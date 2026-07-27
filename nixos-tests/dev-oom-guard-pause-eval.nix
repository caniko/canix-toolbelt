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
        config.canix-toolbelt.services.devOomGuard = {
          enable = true;
          pausePath = "/run/user/1000/canix/foreground-active";
        };
      }
    ];
    specialArgs = {inherit pkgs;};
  };

  cfg = evaluated.config.canix-toolbelt.services.devOomGuard;
  serviceConfig = evaluated.config.systemd.user.services.dev-oom-guard.serviceConfig;
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
        name = "restart-state-is-persistent";
        assertion =
          serviceConfig.StateDirectory
          == "dev-oom-guard"
          && serviceConfig.StateDirectoryMode == "0700";
        message = "the guard must retain PID start-times across service restarts";
      }
    ];
  }
