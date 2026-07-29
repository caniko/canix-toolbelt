{pkgs}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  evaluated = lib.evalModules {
    modules = [
      ../modules/nixos/services/dev-oom-guard.nix
      {
        options.environment.etc = lib.mkOption {
          type = lib.types.attrs;
          default = {};
        };
        options.security.wrappers = lib.mkOption {
          type = lib.types.attrs;
          default = {};
        };
        options.systemd.user.services = lib.mkOption {
          type = lib.types.attrs;
          default = {};
        };
        options.systemd.user.settings.Manager.DefaultOOMPolicy = lib.mkOption {
          type = lib.types.str;
        };
        config.canix-toolbelt.services.devOomGuard = {
          enable = true;
          pausePath = "/run/user/1000/canix/foreground-active";
          protect = [
            {
              name = "interactive-root";
              cmdlineRegex = "interactive-root";
              maxAdj = -1000;
            }
          ];
        };
      }
    ];
    specialArgs = {inherit pkgs;};
  };

  cfg = evaluated.config.canix-toolbelt.services.devOomGuard;
  serviceConfig = evaluated.config.systemd.user.services.dev-oom-guard.serviceConfig;
  managerSettings = evaluated.config.systemd.user.settings.Manager;
  configSource = evaluated.config.environment.etc."dev-oom-guard/config.json".source;
in
  mkEvalCheck {
    name = "dev-oom-guard-pause-eval";
    resultMessage = "dev-oom-guard foreground pause contract evaluated correctly";
    assertions = [
      {
        name = "pause-path-is-configured";
        assertion = cfg.pausePath == "/run/user/1000/canix/foreground-active";
        message = "the marker path must be retained in the public module option";
      }
      {
        name = "pause-path-is-rendered";
        assertion = lib.hasInfix "pausePath" (builtins.readFile configSource);
        message = "the runtime JSON must carry pausePath to the guard process";
      }
      {
        name = "negative-protection-is-rendered";
        assertion = lib.hasInfix ''"maxAdj": -1000'' (builtins.readFile configSource);
        message = "negative protected-process adjustments must reach the runtime config";
      }
      {
        name = "user-scope-oom-policy";
        assertion = managerSettings.DefaultOOMPolicy == "continue";
        message = "the guard must keep a descendant OOM kill from stopping the enclosing user scope";
      }
      {
        name = "restart-state-is-persistent";
        assertion =
          serviceConfig.StateDirectory
          == "dev-oom-guard"
          && serviceConfig.StateDirectoryMode == "0700";
        message = "the guard must retain PID start-times across service restarts";
      }
    ];
  }
