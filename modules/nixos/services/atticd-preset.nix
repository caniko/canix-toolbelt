{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.services.atticd;
in {
  options.canix-toolbelt.services.atticd = {
    enable = lib.mkEnableOption "Attic binary cache server";

    port = lib.mkOption {
      type = lib.types.port;
      default = 5500;
      description = "Port for the Attic server to listen on.";
    };

    storagePath = lib.mkOption {
      type = lib.types.path;
      description = "Path to store cached NARs.";
    };

    secretName = lib.mkOption {
      type = lib.types.str;
      default = "atticd-jwt-secret";
      description = "Name of the age secret containing the atticd JWT environment file.";
    };

    jwtSecretFile = lib.mkOption {
      type = lib.types.path;
      default = config.age.secrets.${cfg.secretName}.path or "";
      defaultText = lib.literalExpression "config.age.secrets.\${config.canix-toolbelt.services.atticd.secretName}.path";
      description = "Path to environment file containing ATTIC_SERVER_TOKEN_RS256_SECRET_BASE64.";
    };

    retentionPeriod = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Default Attic cache retention period, such as \"6 months\".";
    };

    databaseUrl = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "postgresql:///atticd?host=/run/postgresql";
      description = ''
        Database URL for atticd. When null, atticd uses its built-in SQLite
        default. SQLite serializes writes and chokes under parallel pushes,
        so prefer a postgresql:// URL for any host that backs concurrent
        clients.
      '';
    };

    databaseProvisionLocally = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Ensure the local PostgreSQL `atticd` database and role when
        {option}`databaseUrl` points at the local PostgreSQL service.
      '';
    };

    memoryHigh = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "12G";
      description = ''
        systemd MemoryHigh for atticd. A soft cap: when atticd exceeds it,
        the kernel aggressively reclaims pages from the cgroup. atticd has
        no built-in concurrency knobs, so this is the main lever against a
        parallel-push storm.
      '';
    };

    memoryMax = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "20G";
      description = ''
        systemd MemoryMax for atticd. A hard cap: exceeding it kills atticd
        (which then restarts) rather than letting the host OOM. Set well
        above memoryHigh.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = !cfg.databaseProvisionLocally || (cfg.databaseUrl != null && lib.hasPrefix "postgresql:" cfg.databaseUrl && config.services.postgresql.enable);
        message = "canix-toolbelt.services.atticd.databaseProvisionLocally requires a local postgresql:// databaseUrl and services.postgresql.enable = true.";
      }
    ];

    services.postgresql = lib.mkIf cfg.databaseProvisionLocally {
      ensureDatabases = ["atticd"];
      ensureUsers = [
        {
          name = "atticd";
          ensureDBOwnership = true;
        }
      ];
    };

    services.atticd = {
      enable = true;
      environmentFile = cfg.jwtSecretFile;

      settings =
        {
          listen = "[::]:${toString cfg.port}";

          storage = {
            type = "local";
            path = cfg.storagePath;
          };
        }
        // lib.optionalAttrs (cfg.retentionPeriod != null) {
          garbage-collection.default-retention-period = cfg.retentionPeriod;
        }
        // lib.optionalAttrs (cfg.databaseUrl != null) {
          database.url = cfg.databaseUrl;
        };
    };

    # Override DynamicUser for persistent storage on host-managed data
    # mounts; stable ownership avoids surprises across reboots.
    systemd.services.atticd =
      {
        unitConfig.RequiresMountsFor = cfg.storagePath;
        serviceConfig =
          {
            DynamicUser = lib.mkForce false;
            Restart = lib.mkDefault "on-failure";
            RestartSec = lib.mkDefault "5s";
          }
          // lib.optionalAttrs (cfg.memoryHigh != null) {MemoryHigh = cfg.memoryHigh;}
          // lib.optionalAttrs (cfg.memoryMax != null) {MemoryMax = cfg.memoryMax;};
      }
      // lib.optionalAttrs (cfg.databaseUrl != null && lib.hasPrefix "postgresql:" cfg.databaseUrl) {
        after = ["postgresql.service"];
        requires = ["postgresql.service"];
      };

    users.users.atticd = {
      isSystemUser = true;
      group = "atticd";
    };
    users.groups.atticd = {};

    systemd.tmpfiles.rules = [
      "d ${cfg.storagePath} 0750 atticd atticd -"
    ];
  };
}
