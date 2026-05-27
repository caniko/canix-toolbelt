{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.services.foundryVtt;
in
  # Source: https://github.com/StealthBadger747/nix-multi-arch-lab/blob/7fd8d723c561ba9ac151df315ed9f8af8e0ea15e/modules/hosts/oracle-cloud/free-aarch64.nix
  {
    options.canix-toolbelt.services.foundryVtt = {
      enable = lib.mkEnableOption "Foundry VTT server";

      data-path = lib.mkOption {
        type = lib.types.path;
        description = "Path to the directory to store Foundry VTT data";
      };

      timezone = lib.mkOption {
        type = lib.types.str;
        default = "Europe/Berlin";
        description = "Timezone for the Foundry VTT server";
      };

      port = lib.mkOption {
        type = lib.types.port;
        description = "Port to run the Foundry VTT server on";
      };

      hostname = lib.mkOption {
        type = lib.types.str;
        description = "Hostname for the Foundry VTT server";
      };

      account-password-file = lib.mkOption {
        type = lib.types.path;
        description = "Path to a file containing the Foundry VTT account password";
      };

      admin-key-file = lib.mkOption {
        type = lib.types.path;
        description = "Path to a file containing the Foundry VTT admin key";
      };

      version = lib.mkOption {
        type = lib.types.str;
        description = "Foundry VTT major version tag for the container image";
      };

      usernameEnv = lib.mkOption {
        type = lib.types.str;
        description = "FOUNDRY_USERNAME environment value for Foundry VTT account login.";
      };

      uid = lib.mkOption {
        type = lib.types.int;
        default = 421;
        description = "UID for the foundryvtt system user.";
      };

      gid = lib.mkOption {
        type = lib.types.int;
        default = 421;
        description = "GID for the foundryvtt system group.";
      };
    };

    config = lib.mkIf cfg.enable {
      users.users.foundryvtt = {
        isSystemUser = true;
        group = "foundryvtt";
        inherit (cfg) uid;
      };
      users.groups.foundryvtt.gid = cfg.gid;

      systemd.tmpfiles.rules = [
        "d ${cfg.data-path} 0755 foundryvtt foundryvtt -"
      ];

      systemd.services.podman-foundryVtt.preStart = lib.mkAfter ''
        ${pkgs.coreutils}/bin/mkdir -p /run/foundryvtt
        echo "FOUNDRY_PASSWORD=$(${pkgs.coreutils}/bin/cat ${cfg.account-password-file})" > /run/foundryvtt/secrets.env
        echo "FOUNDRY_ADMIN_KEY=$(${pkgs.coreutils}/bin/cat ${cfg.admin-key-file})" >> /run/foundryvtt/secrets.env
      '';

      virtualisation.oci-containers.containers.foundryVtt = {
        image = "docker.io/felddy/foundryvtt:${cfg.version}";
        autoStart = true;
        ports = ["0.0.0.0:${toString cfg.port}:30000"];
        volumes = ["${cfg.data-path}:/data"];
        environment = {
          CONTAINER_PRESERVE_CONFIG = "false";
          FOUNDRY_USERNAME = cfg.usernameEnv;
          FOUNDRY_HOSTNAME = cfg.hostname;
          FOUNDRY_PROXY_SSL = "true";
          FOUNDRY_PROXY_PORT = "443";
          FOUNDRY_COMPRESS_WEBSOCKET = "true";
          FOUNDRY_MINIFY_STATIC_FILES = "true";
          FOUNDRY_IP_DISCOVERY = "false";
          FOUNDRY_UPNP = "false";
          FOUNDRY_TELEMETRY = "true";
          TIMEZONE = cfg.timezone;
        };
        environmentFiles = ["/run/foundryvtt/secrets.env"];
        extraOptions = [
          "--no-healthcheck"
          "--user=${toString cfg.uid}:${toString cfg.gid}"
        ];
      };
    };
  }
