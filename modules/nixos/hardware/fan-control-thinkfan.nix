# Thin wrapper around services.thinkfan that pre-fills hwmon = "/sys/class/hwmon"
# for every sensor and fan entry. Hosts pass curves and chip names; this module
# assembles the thinkfan settings block.
{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.hardware.fanControl;

  hwmon = "/sys/class/hwmon";

  sensorType = lib.types.submodule {
    options = {
      name = lib.mkOption {
        type = lib.types.str;
        description = "hwmon driver/chip name (e.g. \"k10temp\", \"nct6799\", \"it8622\").";
      };
      indices = lib.mkOption {
        type = lib.types.listOf lib.types.int;
        description = "Hwmon temp/pwm indices to read or drive.";
      };
    };
  };

  fanType = lib.types.submodule {
    options = {
      name = lib.mkOption {type = lib.types.str;};
      indices = lib.mkOption {type = lib.types.listOf lib.types.int;};
      levels = lib.mkOption {
        type = lib.types.listOf (lib.types.listOf lib.types.int);
        description = "thinkfan step table: [pwm low high].";
      };
    };
  };

  withHwmon = entry: {inherit hwmon;} // entry;
in {
  options.canix-toolbelt.hardware.fanControl = {
    enable = lib.mkEnableOption "thinkfan-driven hwmon fan control";

    sensors = lib.mkOption {
      type = lib.types.listOf sensorType;
      default = [];
    };

    fans = lib.mkOption {
      type = lib.types.listOf fanType;
      default = [];
    };
  };

  config = lib.mkIf cfg.enable {
    services.thinkfan = {
      enable = true;
      settings = {
        sensors = map withHwmon cfg.sensors;
        fans = map withHwmon cfg.fans;
      };
    };
  };
}
