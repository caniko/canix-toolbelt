{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.opencodeJev;
  routes = builtins.listToAttrs (lib.concatMap (gateway:
    map (path: {
      name = "${lib.removeSuffix "/" gateway.upstream}/${path}";
      value = "http://127.0.0.1:${toString gateway.port}/v1/${path}";
    })
    gateway.paths) (builtins.attrValues cfg.gateways));
in {
  options.canix-toolbelt.opencodeJev = {
    enable = lib.mkEnableOption "Jev routing for OpenCode V2";
    package = lib.mkOption {type = lib.types.package;};
    credentialFile = lib.mkOption {
      type = lib.types.str;
      description = "Runtime TypeSafe credential path; systemd specifiers are supported.";
    };
    gateways = lib.mkOption {
      default = {};
      type = lib.types.attrsOf (lib.types.submodule {
        options = {
          upstream = lib.mkOption {type = lib.types.str;};
          port = lib.mkOption {type = lib.types.port;};
          paths = lib.mkOption {type = lib.types.listOf lib.types.str;};
          requires = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [];
          };
        };
      });
    };
    units = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      readOnly = true;
      default = map (name: "jev-gateway-${name}.service") (builtins.attrNames cfg.gateways);
    };
  };
  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = config.programs.opencode.enable;
        message = "opencodeJev requires programs.opencode.enable.";
      }
      {
        assertion = builtins.length (lib.unique (map (g: g.port) (builtins.attrValues cfg.gateways))) == builtins.length (builtins.attrNames cfg.gateways);
        message = "opencodeJev gateway ports must be unique.";
      }
    ];
    programs.opencode.settings.plugins = lib.mkAfter [
      {
        package = toString ../../runtime/opencode-jev;
        options = {inherit routes;};
      }
    ];
    systemd.user.services = lib.mapAttrs' (name: gateway:
      lib.nameValuePair "jev-gateway-${name}" {
        Unit = {
          Description = "Jev Gateway for OpenCode (${name})";
          Requires = gateway.requires;
          After = gateway.requires;
        };
        Service = {
          ExecStart = "${lib.getExe pkgs.nodejs} --import ${../../runtime/opencode-jev/credential.mjs} ${cfg.package}/share/jev-gateway/dist/index.js";
          LoadCredential = "typesafe:${cfg.credentialFile}";
          Environment = [
            "HOST=127.0.0.1"
            "PORT=${toString gateway.port}"
            "UPSTREAM_BASE_URL=${lib.removeSuffix "/" gateway.upstream}"
            "JEV_DIRECT_CALLS=false"
            "JEV_CLIENT=opencode-v2"
          ];
          Restart = "on-failure";
          RestartSec = "2s";
          UMask = "0077";
          NoNewPrivileges = true;
          KillMode = "control-group";
        };
        Install.WantedBy = ["default.target"];
      })
    cfg.gateways;
  };
}
