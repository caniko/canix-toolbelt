{fleetixLib}: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.opencodeClaude;
  endpoint = name:
    fleetixLib.services.resolveEndpoint {
      inherit (cfg) topology;
      endpointName = name;
      ingressHost = cfg.hostName;
    };
  meridian = endpoint cfg.meridianEndpoint;
  gateway = endpoint cfg.jevEndpoint;
  origin = "http://127.0.0.1:${toString meridian.port}";
in {
  imports = [./opencode-jev.nix];
  options.canix-toolbelt.opencodeClaude = {
    enable = lib.mkEnableOption "Claude subscription integration in OpenCode V2";
    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ../../nix/opencode-with-claude {};
    };
    topology = lib.mkOption {type = lib.types.attrs;};
    hostName = lib.mkOption {type = lib.types.str;};
    meridianEndpoint = lib.mkOption {type = lib.types.str;};
    jevEndpoint = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Optional Jev endpoint; null connects OpenCode directly to Meridian.";
    };
    claudeConfigDirectory = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Optional Claude config override; null uses this user's standard HOME login.";
    };
    requiredUnits = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      readOnly = true;
      default = lib.optional cfg.enable "meridian-opencode.service";
    };
  };
  config = lib.mkIf cfg.enable {
    assertions =
      [
        {
          assertion = config.programs.opencode.enable;
          message = "opencodeClaude requires OpenCode V2.";
        }
        {
          assertion = meridian.bind == "loopback" && meridian.transport == "http" && meridian.targetHost == cfg.hostName;
          message = "opencodeClaude endpoints must be HTTP loopback endpoints on the current host.";
        }
      ]
      ++ lib.optionals (cfg.jevEndpoint != null) [
        {
          assertion = config.canix-toolbelt.opencodeJev.enable;
          message = "An opencodeClaude Jev endpoint requires opencodeJev.enable.";
        }
        {
          assertion = gateway.bind == "loopback" && gateway.transport == "http" && gateway.targetHost == cfg.hostName;
          message = "The opencodeClaude Jev endpoint must be HTTP loopback on the current host.";
        }
        {
          assertion = meridian.port != gateway.port;
          message = "Meridian and Jev must use distinct ports.";
        }
      ];
    programs.opencode.settings = {
      providers.anthropic.settings = {
        baseURL = "${origin}/v1";
        apiKey = "meridian-subscription";
      };
      plugins = lib.mkBefore [
        {
          package = "${cfg.package}/lib/node_modules/opencode-with-claude";
          options.externalBaseURL = origin;
        }
      ];
    };
    canix-toolbelt.opencodeJev.gateways = lib.optionalAttrs (cfg.jevEndpoint != null) {
      anthropic = {
        upstream = "${origin}/v1";
        inherit (gateway) port;
        paths = ["messages"];
        requires = ["meridian-opencode.service"];
      };
    };
    systemd.user.services.meridian-opencode = {
      Unit.Description = "Meridian Claude subscription backend for OpenCode";
      Service = {
        ExecStart = "${cfg.package}/bin/meridian";
        Environment =
          [
            "HOME=${config.home.homeDirectory}"
            "MERIDIAN_HOST=127.0.0.1"
            "MERIDIAN_PORT=${toString meridian.port}"
            "MERIDIAN_PASSTHROUGH=true"
            "MERIDIAN_DEFAULT_AGENT=opencode"
            "MERIDIAN_CONFIG_DIR=${config.xdg.configHome}/meridian-opencode"
          ]
          ++ lib.optional (cfg.claudeConfigDirectory != null) "CLAUDE_CONFIG_DIR=${cfg.claudeConfigDirectory}";
        UnsetEnvironment =
          [
            "ANTHROPIC_API_KEY"
            "ANTHROPIC_BASE_URL"
            "ANTHROPIC_AUTH_TOKEN"
            "CLAUDE_CODE_OAUTH_TOKEN"
            "MERIDIAN_WORKDIR"
            "CLAUDE_PROXY_WORKDIR"
          ]
          ++ lib.optional (cfg.claudeConfigDirectory == null) "CLAUDE_CONFIG_DIR";
        WorkingDirectory = config.home.homeDirectory;
        Restart = "on-failure";
        RestartSec = "2s";
        KillMode = "control-group";
        TimeoutStopSec = "15s";
        UMask = "0077";
        NoNewPrivileges = true;
      };
      Install.WantedBy = ["default.target"];
    };
  };
}
