{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.networking.wireguard-status;
  statusTarget = lib.removePrefix "/etc/" cfg.statusFile;
  validStatusFile = lib.hasPrefix "/etc/" cfg.statusFile && statusTarget != "";
in {
  options.canix-toolbelt.networking.wireguard-status = {
    enable = lib.mkEnableOption "the declarative WireGuard status exporter";

    interfaces = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "WireGuard interfaces to expose; an empty list exposes all interfaces.";
    };

    listenAddress = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1";
      description = "Address on which the local status metrics endpoint listens.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 9586;
      description = "Port for the local WireGuard status metrics endpoint.";
    };

    withRemoteIp = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Expose each peer's remote endpoint in the metrics.";
    };

    statusFile = lib.mkOption {
      type = lib.types.str;
      default = "/etc/canix-toolbelt/wireguard-status.json";
      description = "Path under /etc where the JSON status descriptor is written.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = validStatusFile;
        message = "canix-toolbelt.networking.wireguard-status.statusFile must be a non-empty path under /etc/ (got ${cfg.statusFile}).";
      }
    ];

    services.prometheus.exporters.wireguard = {
      enable = true;
      inherit (cfg) interfaces listenAddress port withRemoteIp;
    };

    environment.etc = lib.mkIf validStatusFile {
      ${statusTarget}.text = builtins.toJSON {
        schemaVersion = 1;
        metricsUrl = "http://${cfg.listenAddress}:${toString cfg.port}/metrics";
        inherit (cfg) interfaces;
      };
    };
  };
}
