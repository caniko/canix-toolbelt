# Declaratively seed Stalwart Mail Server accounts (create-if-missing).
#
# Existing accounts are left untouched — this module never overwrites passwords
# or settings changed via the admin UI. Remove the account from Nix to stop
# seeding it; deletion must be done in the UI/API.
#
# Per-account disabledPermissions can seed restricted principals, such as
# send-only service identities with email-receive disabled.
#
# Stalwart's management API is reached over its HTTP listener (default
# 127.0.0.1:8080 in upstream defaults, often 127.0.0.1:8580 in our deployments).
# Auth is HTTP Basic with the admin principal.
{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mapAttrs' mkEnableOption mkIf mkOption nameValuePair types;
  cfg = config.canix-toolbelt.services.stalwartSeedAccounts;
  shellLiteral = value: "${lib.escapeShellArg value}''";

  accountSubmodule = types.submodule {
    options = {
      emails = mkOption {
        type = types.listOf types.str;
        description = "Email addresses bound to the account. First entry is primary.";
        example = ["alice@example.com" "a@example.com"];
      };
      passwordFile = mkOption {
        type = types.path;
        description = "File containing the plain-text password (e.g. an agenix secret path).";
      };
      description = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional display name.";
      };
      quota = mkOption {
        type = types.nullOr types.int;
        default = null;
        description = "Optional mailbox quota in bytes.";
      };
      disabledPermissions = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Stalwart permission identifiers to disable on this principal (create-time only). e.g. [\"email-receive\"] for send-only.";
        example = ["email-receive"];
      };
    };
  };

  accountManifest = pkgs.writeText "stalwart-seed-accounts.json" (
    builtins.toJSON (mapAttrs' (n: a:
      nameValuePair n {
        inherit (a) emails description quota disabledPermissions;
        passwordPath = toString a.passwordFile;
      })
    cfg.accounts)
  );

  # Stalwart rejects creating an individual whose email is on a domain that has
  # no domain principal ({"error":"notFound","item":"<domain>"}), and never
  # auto-creates it. Derive the domain set from every account email (plus any
  # explicit extraDomains) and seed those domain principals BEFORE the accounts.
  emailDomain = email: lib.last (lib.splitString "@" email);
  accountDomains = lib.concatMap (a: map emailDomain a.emails) (lib.attrValues cfg.accounts);
  allDomains = lib.unique (accountDomains ++ cfg.extraDomains);
  domainManifest = pkgs.writeText "stalwart-seed-domains.json" (builtins.toJSON allDomains);

  seedScript = pkgs.writeShellApplication {
    name = "stalwart-seed-accounts";
    runtimeInputs = with pkgs; [curl jq coreutils];
    text = ''
      set -euo pipefail

      endpoint=${shellLiteral cfg.endpoint}
      admin_user=${shellLiteral cfg.adminUser}
      admin_pw=$(cat ${lib.escapeShellArg cfg.adminPasswordFile})
      auth=$(printf '%s:%s' "$admin_user" "$admin_pw" | base64 -w0)

      # Domain principals first — Stalwart needs them before any account on the
      # domain. Same create-if-missing + body-parse existence check as accounts.
      mapfile -t domains < <(jq -r '.[]' ${domainManifest})
      for dom in "''${domains[@]}"; do
        resp=$(curl -sS -w $'\n%{http_code}' \
          -H "Authorization: Basic $auth" \
          "$endpoint/api/principal/$dom" || printf '\n000')
        status=$(printf '%s' "$resp" | tail -n1)
        rbody=$(printf '%s' "$resp" | sed '$d')

        if [ "$status" != "200" ] && [ "$status" != "404" ]; then
          echo "[stalwart-seed] unexpected status $status for domain $dom; aborting" >&2
          exit 1
        fi
        if printf '%s' "$rbody" | jq -e '.data != null and (.error // empty) == ""' >/dev/null 2>&1; then
          echo "[stalwart-seed] domain $dom exists, skipping" >&2
          continue
        fi

        echo "[stalwart-seed] creating domain $dom" >&2
        curl -fsS -X POST \
          -H "Authorization: Basic $auth" \
          -H "Content-Type: application/json" \
          --data "$(jq -n --arg n "$dom" '{type:"domain", name:$n}')" \
          "$endpoint/api/principal" >/dev/null
      done

      mapfile -t accounts < <(jq -r 'keys[]' ${accountManifest})

      for name in "''${accounts[@]}"; do
        emails=$(jq -c --arg n "$name" '.[$n].emails' ${accountManifest})
        desc=$(jq -r --arg n "$name" '.[$n].description // empty' ${accountManifest})
        quota=$(jq -r --arg n "$name" '.[$n].quota // empty' ${accountManifest})
        disabled_permissions=$(jq -c --arg n "$name" '.[$n].disabledPermissions' ${accountManifest})
        pw_path=$(jq -r --arg n "$name" '.[$n].passwordPath' ${accountManifest})
        password=$(cat "$pw_path")

        # The principal NAME is the SASL login (Stalwart resolves AUTH usernames
        # strictly by principal name via NameToId, never by email alias). Use the
        # lowercased primary email as the name so services authenticate with the
        # full address (e.g. noreply@example.com), which is what mail clients send.
        login=$(printf '%s' "$emails" | jq -r '.[0] | ascii_downcase')

        # Stalwart's GET /api/principal/<name> returns HTTP 200 even when the
        # principal is missing — signalling absence in the JSON body as
        # {"error":"notFound"} — so the HTTP status alone cannot decide
        # existence. Fetch the body and inspect it: a present principal returns
        # {"data":{...}}; a missing one returns {"error":"notFound",...}. (HTTP
        # errors like 5xx still surface via the status check below.)
        resp=$(curl -sS -w $'\n%{http_code}' \
          -H "Authorization: Basic $auth" \
          "$endpoint/api/principal/$login" || printf '\n000')
        status=$(printf '%s' "$resp" | tail -n1)
        rbody=$(printf '%s' "$resp" | sed '$d')

        if [ "$status" != "200" ] && [ "$status" != "404" ]; then
          echo "[stalwart-seed] unexpected status $status for $login; aborting" >&2
          exit 1
        fi

        # Present iff the body carries a "data" object (not an error).
        if printf '%s' "$rbody" | jq -e '.data != null and (.error // empty) == ""' >/dev/null 2>&1; then
          echo "[stalwart-seed] $login exists, skipping" >&2
          continue
        fi

        body=$(jq -n \
          --arg n "$login" \
          --arg d "$desc" \
          --arg pw "$password" \
          --arg q "$quota" \
          --argjson em "$emails" \
          --argjson dp "$disabled_permissions" \
          '{type:"individual", name:$n, emails:$em, secrets:[$pw]}
            + (if $d  != "" then {description:$d}        else {} end)
            + (if $q  != "" then {quota:($q|tonumber)}   else {} end)
            + (if ($dp|length) > 0 then {disabledPermissions:$dp} else {} end)')

        echo "[stalwart-seed] creating $login" >&2
        curl -fsS -X POST \
          -H "Authorization: Basic $auth" \
          -H "Content-Type: application/json" \
          --data "$body" \
          "$endpoint/api/principal" >/dev/null
      done
    '';
  };
in {
  options.canix-toolbelt.services.stalwartSeedAccounts = {
    enable = mkEnableOption "declarative Stalwart account seeding (create-if-missing)";

    endpoint = mkOption {
      type = types.str;
      default = "http://127.0.0.1:8080";
      example = "http://127.0.0.1:8580";
      description = "Stalwart management API base URL (no trailing slash).";
    };

    adminUser = mkOption {
      type = types.str;
      default = "admin";
      description = "Admin principal used for API auth.";
    };

    adminPasswordFile = mkOption {
      type = types.path;
      description = "File containing the admin password (e.g. an agenix secret path).";
    };

    accounts = mkOption {
      type = types.attrsOf accountSubmodule;
      default = {};
      description = ''
        Accounts to seed. The attribute name is the principal login name.
        Existing accounts (matched by name) are left alone — this never overwrites.
      '';
    };

    extraDomains = mkOption {
      type = types.listOf types.str;
      default = [];
      example = ["tartanoglu.com"];
      description = ''
        Additional mail domains to seed as Stalwart domain principals, on top of
        the domains derived from every account email. Stalwart rejects creating
        an account whose email is on a domain with no domain principal, so all
        such domains are created (create-if-missing) before the accounts.
      '';
    };

    serviceAfter = mkOption {
      type = types.listOf types.str;
      default = ["stalwart.service"];
      description = ''
        Units the seed waits for (after + requires) before running. Defaults to
        the nixpkgs Stalwart daemon unit `stalwart.service`. NOTE: a hard
        requires on a non-existent unit makes systemd silently refuse the start
        job, so this must name the real daemon unit on the host.
      '';
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.accounts == {} || cfg.adminPasswordFile != null;
        message = "canix-toolbelt.services.stalwartSeedAccounts.adminPasswordFile must be set when accounts are declared.";
      }
    ];

    systemd.services.stalwart-seed-accounts = {
      description = "Seed declarative Stalwart accounts (create-if-missing)";
      after = cfg.serviceAfter;
      requires = cfg.serviceAfter;
      wantedBy = ["multi-user.target"];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${seedScript}/bin/stalwart-seed-accounts";
        # Retry on transient failures (Stalwart still warming up etc.)
        Restart = "on-failure";
        RestartSec = "10s";
      };
      unitConfig.StartLimitBurst = 6;
      unitConfig.StartLimitIntervalSec = 300;
    };
  };
}
