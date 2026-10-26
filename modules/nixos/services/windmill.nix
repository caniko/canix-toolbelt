# Windmill (CE) on top of the nixpkgs module: loopback-only server, sized
# default worker group and a working native worker group.
{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.services.windmill;
  inherit (lib) mkEnableOption mkIf mkOption types;
  # Workers reach the server API through BASE_INTERNAL_URL; the upstream
  # default (localhost:8000) does not follow serverPort.
  internal = "http://127.0.0.1:${toString cfg.port}";
in {
  options.canix-toolbelt.services.windmill = {
    enable = mkEnableOption "Windmill server and workers";

    baseUrl = mkOption {
      type = types.str;
      example = "https://windmill.example.com";
      description = "Public URL (WM_BASE_URL); also the OAuth and MCP origin.";
    };

    port = mkOption {
      type = types.port;
      default = 8001;
      description = "Loopback port of the Windmill API/UI server.";
    };

    workers = mkOption {
      type = types.ints.positive;
      default = 1;
      description = "Concurrent job slots of the default worker group (NUM_WORKERS).";
    };

    nativeWorkers = mkOption {
      type = types.bool;
      default = true;
      description = "Run the native worker group (8 subworkers for HTTP/SQL jobs).";
    };

    environment = mkOption {
      type = types.attrsOf types.str;
      default = {};
      description = "Extra environment shared by the server and all workers.";
    };
  };

  config = mkIf cfg.enable {
    services.windmill = {
      enable = true;
      inherit (cfg) baseUrl;
      serverPort = cfg.port;
      database.createLocally = true;
    };

    systemd.services = {
      windmill-server.environment =
        cfg.environment
        // {
          SERVER_BIND_ADDR = "127.0.0.1";
          BASE_INTERNAL_URL = internal;
        };
      windmill-worker.environment =
        cfg.environment
        // {
          BASE_INTERNAL_URL = internal;
          NUM_WORKERS = toString cfg.workers;
        };
      # ponytail: nixpkgs labels this unit "native" but never sets NATIVE_MODE,
      # so it runs as a plain single worker; drop once fixed upstream.
      windmill-worker-native =
        if cfg.nativeWorkers
        then {
          environment =
            cfg.environment
            // {
              BASE_INTERNAL_URL = internal;
              NATIVE_MODE = "true";
            };
        }
        else {enable = false;};
    };
  };
}
