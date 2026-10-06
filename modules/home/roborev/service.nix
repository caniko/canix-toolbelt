{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.roborev;
  svc = config.services.roborev;
  runtimePath = value:
    builtins.isString value
    && lib.hasPrefix "/" value
    && value != "/nix/store"
    && !lib.hasPrefix "/nix/store/" value
    && !lib.hasInfix "//" value
    && !lib.hasInfix "/../" "${value}/"
    && !lib.hasInfix "/./" "${value}/"
    && builtins.match ".*[[:cntrl:]].*" value == null;
  # Exec directives expand dollars and specifiers after C-style unquoting.
  argument = value: "\"${lib.replaceStrings ["\\" "\"" "%" "$"] ["\\\\" "\\\"" "%%" "$$"] value}\"";
  command = args: lib.concatMapStringsSep " " argument args;
  # Environment does not perform Exec-style dollar expansion.
  environment = name: value: "\"${name}=${lib.replaceStrings ["\\" "\"" "%"] ["\\\\" "\\\"" "%%"] value}\"";
  # EnvironmentFile consumes one whole path, without C/shell unquoting. Quotes
  # would become part of the filename. Escape specifiers and the backslash used
  # by systemd's pathname globbing, not Exec-style C/shell quoting.
  environmentFilePath = value:
    runtimePath value
    && value == lib.trim value
    && !lib.hasSuffix "\\" value
    && builtins.all (character: !lib.hasInfix character value) ["*" "?" "[" "]"];
in {
  options.services.roborev = {
    enable = lib.mkEnableOption "the foreground roborev user daemon";
    environmentFiles = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "Required systemd environment files; absolute runtime strings outside the store. Restart explicitly after rotating their contents.";
    };
    requiredFiles = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "Readable runtime prerequisites, such as a GitHub App key; never file contents or Nix path literals.";
    };
  };
  config = lib.mkMerge [
    (lib.mkIf svc.enable {
      programs.roborev.enable = lib.mkDefault true;
      assertions = [
        {
          assertion = cfg.enable;
          message = "services.roborev requires programs.roborev.enable.";
        }
        {
          assertion = pkgs.stdenv.hostPlatform.isLinux && config.systemd.user.enable;
          message = "services.roborev requires Linux and a systemd user manager.";
        }
        {
          assertion = builtins.all runtimePath (svc.environmentFiles ++ svc.requiredFiles);
          message = "roborev runtime files must be absolute string paths outside /nix/store without control characters.";
        }
        {
          assertion = builtins.all environmentFilePath svc.environmentFiles;
          message = "roborev environment files require literal paths without globs, trailing whitespace or a continuation backslash.";
        }
      ];
    })
    (lib.mkIf (svc.enable && cfg.enable) {
      systemd.user.services.roborev = {
        Unit = {
          Description = "Roborev code review daemon";
          X-Restart-Triggers = [cfg._configFile cfg.finalPackage];
          StartLimitIntervalSec = 300;
          StartLimitBurst = 5;
        };
        Service = {
          Type = "notify";
          NotifyAccess = "main";
          ExecStartPre =
            [
              (command ["${pkgs.coreutils}/bin/test" "-r" "${cfg.dataDir}/config.toml"])
            ]
            ++ (lib.concatMap (path: [
                (command ["${pkgs.coreutils}/bin/test" "-f" path])
                (command ["${pkgs.coreutils}/bin/test" "-x" path])
              ])
              (builtins.attrValues cfg.agentCommands))
            ++ (lib.concatMap (path: [
                (command ["${pkgs.coreutils}/bin/test" "-f" path])
                (command ["${pkgs.coreutils}/bin/test" "-r" path])
              ])
              svc.requiredFiles)
            ++ [(command ["${cfg.finalPackage}/bin/roborev" "config" "validate" "--global"])];
          ExecStart = command ["${cfg.finalPackage}/bin/roborev" "daemon" "run" "--config" "${cfg.dataDir}/config.toml"];
          Environment = lib.mapAttrsToList environment {
            HOME = config.home.homeDirectory;
            XDG_CONFIG_HOME = config.xdg.configHome;
            XDG_DATA_HOME = config.xdg.dataHome;
            XDG_CACHE_HOME = config.xdg.cacheHome;
            XDG_STATE_HOME = config.xdg.stateHome;
            ROBOREV_DATA_DIR = cfg.dataDir;
            ROBOREV_TELEMETRY_ENABLED =
              if cfg.enableTelemetry
              then "1"
              else "0";
            PATH = cfg._runtimePath;
            GIT_TERMINAL_PROMPT = "0";
          };
          EnvironmentFile = map (path: lib.replaceStrings ["\\" "%"] ["\\\\" "%%"] path) svc.environmentFiles;
          WorkingDirectory = "%h";
          Restart = "on-failure";
          RestartSec = 5;
          TimeoutStartSec = 120;
          TimeoutStopSec = 120;
          KillMode = "control-group";
          UMask = "0077";
          LimitCORE = 0;
          StandardOutput = "journal";
          StandardError = "journal";
          CPUAccounting = true;
          MemoryAccounting = true;
          TasksAccounting = true;
        };
        Install.WantedBy = ["default.target"];
      };
    })
  ];
}
