{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mkIf mkEnableOption mkOption types;
  cfg = config.canix-toolbelt.services.betterdesk-server;
in {
  options.canix-toolbelt.services.betterdesk-server = {
    enable = mkEnableOption "BetterDesk server (RustDesk-compatible signal + relay + API)";

    package = mkOption {
      type = types.package;
      description = "The betterdesk-server package to use";
    };

    wgIp = mkOption {
      type = types.str;
      description = "WireGuard IP of this host, used for OIDC redirect URL and default relay address";
      example = "10.0.0.1";
    };

    oidcClientSecretPath = mkOption {
      type = types.nullOr types.path;
      default = null;
      description = ''
        Path to an OIDC client secret file for the BetterDesk Console API.
        When non-null, a <literal>betterdesk-oidc-setup</literal> oneshot is
        installed that loads the secret as a systemd credential and runs
        <literal>oidcSetupScript</literal>.
      '';
    };

    oidcSetupScript = mkOption {
      type = types.nullOr types.lines;
      default = null;
      description = ''
        Shell script that seeds the BetterDesk OIDC configuration via the
        local API. Runs as a <literal>betterdesk-oidc-setup</literal> oneshot
        after the server starts. The OIDC client secret is available at
        <literal>$CREDENTIALS_DIRECTORY/oidc-client-secret</literal>.
        When null and <literal>oidcClientSecretPath</literal> is set, a
        sensible default is used. Ignored when <literal>oidcClientSecretPath</literal>
        is null.
      '';
    };

    openFirewall = mkOption {
      type = types.bool;
      default = true;
      description = "Open firewall ports for BetterDesk on the WireGuard interface";
    };

    firewallInterface = mkOption {
      type = types.nullOr types.str;
      default = "wg-home";
      description = "Interface to scope firewall rules to, or null for system-wide";
    };
  };

  config = mkIf cfg.enable {
    services.postgresql = {
      enable = true;
      ensureDatabases = ["betterdesk"];
      ensureUsers = [
        {
          name = "betterdesk";
          ensureDBOwnership = true;
        }
      ];
    };

    users.users.betterdesk = {
      isSystemUser = true;
      group = "betterdesk";
      home = "/var/lib/betterdesk";
      createHome = true;
    };
    users.groups.betterdesk = {};

    systemd.services.betterdesk-server = {
      description = "BetterDesk Server — RustDesk-compatible signal + relay + API";
      after = ["postgresql.service" "network.target"];
      wants = ["postgresql.service"];
      wantedBy = ["multi-user.target"];

      preStart = ''
        install -d -m 0700 -o betterdesk -g betterdesk /var/lib/betterdesk
      '';

      serviceConfig = {
        User = "betterdesk";
        Group = "betterdesk";
        StateDirectory = "betterdesk";
        ExecStart = "${cfg.package}/bin/betterdesk-server";
        Restart = "on-failure";
        RestartSec = "5s";
        Environment = [
          "DB_URL=postgresql:///betterdesk?host=/run/postgresql"
          "KEY_FILE=/var/lib/betterdesk/id_ed25519"
          "SIGNAL_PORT=21116"
          "RELAY_PORT=21117"
          "API_PORT=21114"
        ];
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
      };
    };

    systemd.services.betterdesk-oidc-setup = mkIf (cfg.oidcClientSecretPath != null) {
      description = "Seed BetterDesk OIDC config from agenix";
      after = ["betterdesk-server.service"];
      requires = ["betterdesk-server.service"];
      wantedBy = ["multi-user.target"];

      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        User = "betterdesk";
        Group = "betterdesk";
        LoadCredential = [
          "oidc-client-secret:${cfg.oidcClientSecretPath}"
        ];
      };

      script = builtins.replaceStrings ["\n"] ["\n"] (cfg.oidcSetupScript or ''
        set -eu
        api_key_file="/var/lib/betterdesk/.api_key"
        secret_file="$CREDENTIALS_DIRECTORY/oidc-client-secret"
        test -r "$secret_file"
        client_secret=$(cat "$secret_file")
        for i in $(seq 1 30); do
          test -r "$api_key_file" && break
          sleep 1
        done
        test -r "$api_key_file" || { echo "API key not found after 30s"; exit 1; }
        api_key=$(cat "$api_key_file")
        oidc_config=$(${pkgs.jq}/bin/jq -n \
          --arg enabled true \
          --arg display_name "OIDC" \
          --arg issuer_url "" \
          --arg client_id "betterdesk" \
          --arg client_secret "$client_secret" \
          --arg redirect_url "http://${cfg.wgIp}:21114/api/auth/oidc/callback" \
          --arg scopes "openid profile email" \
          --arg auto_discovery true \
          --arg use_pkce true \
          --arg claim_username "preferred_username" \
          --arg claim_email "email" \
          --arg claim_name "name" \
          --arg claim_groups "groups" \
          --arg allow_signup true \
          --arg default_role "viewer" \
          --arg group_role_map "" \
          '{
            enabled: ($enabled | test("true")),
            display_name: $display_name,
            issuer_url: $issuer_url,
            client_id: $client_id,
            client_secret: $client_secret,
            redirect_url: $redirect_url,
            scopes: $scopes,
            auto_discovery: ($auto_discovery | test("true")),
            use_pkce: ($use_pkce | test("true")),
            claim_username: $claim_username,
            claim_email: $claim_email,
            claim_name: $claim_name,
            claim_groups: $claim_groups,
            allow_signup: ($allow_signup | test("true")),
            default_role: $default_role,
            group_role_map: $group_role_map
          }'
        )
        ${pkgs.curl}/bin/curl -s -X PUT http://127.0.0.1:21114/api/auth/oidc/config \
          -H "Authorization: Bearer $api_key" \
          -H "Content-Type: application/json" \
          -d "$oidc_config"
      '');
    };

    networking.firewall = mkIf cfg.openFirewall (
      if cfg.firewallInterface != null
      then {
        interfaces.${cfg.firewallInterface} = {
          allowedTCPPorts = [21114 21115 21116 21117 21118 21119];
          allowedUDPPorts = [21116];
        };
      }
      else {
        allowedTCPPorts = [21114 21115 21116 21117 21118 21119];
        allowedUDPPorts = [21116];
      }
    );
  };
}
