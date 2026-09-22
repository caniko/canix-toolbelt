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
        options.systemd.services.dev-oom-protector = {
          description = lib.mkOption {type = lib.types.str;};
          wantedBy = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [];
          };
          serviceConfig = lib.mkOption {
            type = lib.types.attrs;
            default = {};
          };
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
          protectUsers = ["developer"];
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
  protectorConfig = evaluated.config.systemd.services.dev-oom-protector.serviceConfig;
  managerSettings = evaluated.config.systemd.user.settings.Manager;
  configSource = evaluated.config.environment.etc."dev-oom-guard/config.json".source;
in
  mkEvalCheck {
    name = "dev-oom-guard-pause-eval";
    resultMessage = "dev-oom-guard foreground pause contract evaluated correctly";
    nativeBuildInputs = [pkgs.jq];
    # Inspect generated files at build time, without import-from-derivation.
    runtimeScript = ''
      jq -e --arg pausePath ${lib.escapeShellArg cfg.pausePath} '.pausePath == $pausePath' ${configSource} >/dev/null
      jq -e 'any(.protect[]; .name == "interactive-root" and .maxAdj == -1000)' ${configSource} >/dev/null
    '';
    assertions = [
      {
        name = "pause-path-is-configured";
        assertion = cfg.pausePath == "/run/user/1000/canix/foreground-active";
        message = "the marker path must be retained in the public module option";
      }
      {
        name = "user-scope-oom-policy";
        assertion = managerSettings.DefaultOOMPolicy == "continue";
        message = "the guard must keep a descendant OOM kill from stopping the enclosing user scope";
      }
      {
        name = "root-protector-is-restricted";
        assertion =
          lib.hasInfix " --protect " protectorConfig.ExecStart
          && protectorConfig.NoNewPrivileges == true
          && protectorConfig.ProtectSystem == "strict";
        message = "protected-process adjustment must stay in the restricted root service";
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
