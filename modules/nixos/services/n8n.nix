# n8n (community) on top of the nixpkgs module: loopback-only behind a TLS
# reverse proxy, local PostgreSQL over the Unix socket and external task
# runners. Secrets are file paths loaded through systemd credentials.
{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.services.n8n;
  inherit (lib) mkEnableOption mkIf mkOption types;
  host = lib.removePrefix "https://" cfg.baseUrl;
in {
  options.canix-toolbelt.services.n8n = {
    enable = mkEnableOption "n8n with PostgreSQL and external task runners";

    baseUrl = mkOption {
      type = types.strMatching "https://[^/]+";
      example = "https://n8n.example.com";
      description = "Public origin used for the editor, webhooks and MCP.";
    };

    port = mkOption {
      type = types.port;
      default = 5678;
      description = "Loopback HTTP port.";
    };

    encryptionKeyFile = mkOption {
      type = types.str;
      description = "Credential encryption key. Losing it makes every stored credential unreadable.";
    };

    runnersAuthTokenFile = mkOption {
      type = types.str;
      description = "Shared token between the task broker and the task runners.";
    };

    environment = mkOption {
      type = types.attrsOf types.str;
      default = {};
      description = "Extra n8n environment; `_FILE` variables become systemd credentials.";
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = !(lib.hasPrefix builtins.storeDir cfg.encryptionKeyFile || lib.hasPrefix builtins.storeDir cfg.runnersAuthTokenFile);
        message = "canix-toolbelt.services.n8n: secret files must not live in the Nix store";
      }
    ];

    # The unit runs as DynamicUser "n8n", so socket peer auth maps it to the
    # role of the same name without a password.
    services.postgresql = {
      enable = lib.mkDefault true;
      ensureDatabases = ["n8n"];
      ensureUsers = [
        {
          name = "n8n";
          ensureDBOwnership = true;
        }
      ];
    };

    services.n8n = {
      enable = true;
      taskRunners.enable = true;
      environment =
        {
          N8N_PORT = cfg.port;
          N8N_LISTEN_ADDRESS = "127.0.0.1";
          N8N_HOST = host;
          N8N_PROTOCOL = "https";
          N8N_EDITOR_BASE_URL = "${cfg.baseUrl}/";
          WEBHOOK_URL = "${cfg.baseUrl}/";
          N8N_PROXY_HOPS = "1";
          N8N_ENCRYPTION_KEY_FILE = cfg.encryptionKeyFile;
          N8N_RUNNERS_AUTH_TOKEN_FILE = cfg.runnersAuthTokenFile;
          DB_TYPE = "postgresdb";
          DB_POSTGRESDB_HOST = "/run/postgresql";
          DB_POSTGRESDB_DATABASE = "n8n";
          DB_POSTGRESDB_USER = "n8n";
        }
        // cfg.environment;
    };

    systemd.services.n8n = {
      requires = ["postgresql.target"];
      after = ["postgresql.target"];
    };
  };
}
