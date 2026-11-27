{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mkOption types;
  cfg = config.canix-toolbelt.services.rustHousekeeping;
in {
  imports = [../activation-contracts.nix];

  options.canix-toolbelt.services.rustHousekeeping = {
    enable = lib.mkEnableOption "periodic Cargo cache housekeeping";
    user = mkOption {
      type = types.nonEmptyStr;
      description = "User that runs housekeeping.";
    };
    home = mkOption {
      type = types.path;
      description = "Home directory of the housekeeping user.";
    };
    cargoHome = mkOption {
      type = types.path;
      default = "${toString cfg.home}/.cargo";
      defaultText = lib.literalExpression ''"''${home}/.cargo"'';
      description = "Cargo cache directory.";
    };
    schedule = mkOption {
      type = types.str;
      default = "weekly";
      description = "Systemd calendar expression for the housekeeping timer.";
    };
    randomizedDelaySec = mkOption {
      type = types.str;
      default = "30m";
      description = "Maximum randomized delay for the housekeeping timer.";
    };
    package = mkOption {
      type = types.package;
      default = pkgs.cargo-cache;
      defaultText = lib.literalExpression "pkgs.cargo-cache";
      description = "Package providing the cargo-cache executable.";
    };
  };

  config = lib.mkMerge [
    {canix-toolbelt.activation.contracts.rustHousekeeping.enabled = cfg.enable;}
    (lib.mkIf cfg.enable {
      canix-toolbelt.activation.contracts.rustHousekeeping = {
        requirements.cargo-home = {
          phase = "runtime";
          satisfied = false;
          summary = "The working directory and Cargo home must exist before housekeeping runs.";
          producer = "local filesystem";
          recovery = "Create or mount the directories, then restart the housekeeping timer.";
          validation = "test -d ${lib.escapeShellArg (toString cfg.home)} && test -d ${lib.escapeShellArg (toString cfg.cargoHome)}";
        };
        artifacts = [
          {
            kind = "directory";
            path = toString cfg.cargoHome;
            sensitive = false;
          }
        ];
        units = ["canix-rust-housekeeping.service" "canix-rust-housekeeping.timer"];
      };

      systemd.services.canix-rust-housekeeping = {
        startAt = cfg.schedule;
        description = "Cargo cache housekeeping";
        environment = {
          HOME = toString cfg.home;
          CARGO_HOME = toString cfg.cargoHome;
        };
        serviceConfig = {
          Type = "oneshot";
          User = cfg.user;
          WorkingDirectory = toString cfg.home;
          ExecStart = "${lib.getExe cfg.package} --autoclean";
        };
      };

      systemd.timers.canix-rust-housekeeping = {
        description = "Cargo cache housekeeping";
        timerConfig = {
          Persistent = true;
          RandomizedDelaySec = cfg.randomizedDelaySec;
        };
      };
    })
  ];
}
