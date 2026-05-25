# Declaratively seed Stalwart Mail Server accounts (create-if-missing).
#
# Existing accounts are left untouched — this module never overwrites passwords
# or settings changed via the admin UI. Remove the account from Nix to stop
# seeding it; deletion must be done in the UI/API.
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
    };
  };

  accountManifest = pkgs.writeText "stalwart-seed-accounts.json" (
    builtins.toJSON (mapAttrs' (n: a:
      nameValuePair n {
        inherit (a) emails description quota;
        passwordPath = toString a.passwordFile;
      })
    cfg.accounts)
  );

  seedScript = pkgs.writeShellApplication {
    name = "stalwart-seed-accounts";
    runtimeInputs = with pkgs; [curl jq coreutils];
    text = ''
      set -euo pipefail

      endpoint=${shellLiteral cfg.endpoint}
      admin_user=${shellLiteral cfg.adminUser}
      admin_pw=$(cat ${lib.escapeShellArg cfg.adminPasswordFile})
      auth=$(printf '%s:%s' "$admin_user" "$admin_pw" | base64 -w0)

      mapfile -t accounts < <(jq -r 'keys[]' ${accountManifest})

      for name in "''${accounts[@]}"; do
        emails=$(jq -c --arg n "$name" '.[$n].emails' ${accountManifest})
        desc=$(jq -r --arg n "$name" '.[$n].description // empty' ${accountManifest})
        quota=$(jq -r --arg n "$name" '.[$n].quota // empty' ${accountManifest})
        pw_path=$(jq -r --arg n "$name" '.[$n].passwordPath' ${accountManifest})
        password=$(cat "$pw_path")

        status=$(curl -sS -o /dev/null -w '%{http_code}' \
          -H "Authorization: Basic $auth" \
          "$endpoint/api/principal/$name" || echo "000")

        if [ "$status" = "200" ]; then
          echo "[stalwart-seed] $name exists, skipping" >&2
          continue
        fi
        if [ "$status" != "404" ]; then
          echo "[stalwart-seed] unexpected status $status for $name; aborting" >&2
          exit 1
        fi

        body=$(jq -n \
          --arg n "$name" \
          --arg d "$desc" \
          --arg pw "$password" \
          --arg q "$quota" \
          --argjson em "$emails" \
          '{type:"individual", name:$n, emails:$em, secrets:[$pw]}
            + (if $d  != "" then {description:$d}        else {} end)
            + (if $q  != "" then {quota:($q|tonumber)}   else {} end)')

        echo "[stalwart-seed] creating $name" >&2
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

    serviceAfter = mkOption {
      type = types.listOf types.str;
      default = ["stalwart-mail.service"];
      description = "Units the seed waits for before running.";
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
