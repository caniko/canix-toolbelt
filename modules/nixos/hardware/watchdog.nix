{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.hardware.watchdog;

  chipsetModules = {
    amd = "sp5100_tco";
    intel = "iTCO_wdt";
    rockchip = "dw_wdt";
  };

  inherit (lib) mkEnableOption mkOption types mkIf;
in {
  options.canix-toolbelt.hardware.watchdog = {
    enable = mkEnableOption "hardware watchdog";

    chipset = mkOption {
      type = types.enum ["amd" "intel" "rockchip"];
      description = "Chipset family for kernel watchdog module selection";
    };

    runtimeTime = mkOption {
      type = types.str;
      default = "30s";
      description = "Reboot if systemd doesn't pet the watchdog within this time";
    };

    rebootTime = mkOption {
      type = types.str;
      default = "10min";
      description = "Force reboot if graceful reboot takes longer than this";
    };
  };

  config = mkIf cfg.enable {
    boot.kernelModules = [chipsetModules.${cfg.chipset}];

    systemd.settings.Manager = {
      RuntimeWatchdogSec = cfg.runtimeTime;
      RebootWatchdogSec = cfg.rebootTime;
      KExecWatchdogSec = cfg.rebootTime;
    };
  };
}
