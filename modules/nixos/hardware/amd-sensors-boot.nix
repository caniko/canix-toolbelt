{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.hardware.amdSensorsBoot;
in {
  options.canix-toolbelt.hardware.amdSensorsBoot = {
    enable = lib.mkEnableOption "AMD desktop board sensor boot support";

    forceId = lib.mkOption {
      type = lib.types.str;
      default = "0x8622";
      description = "IT87 force_id value for boards whose sensor chip needs one.";
    };
  };

  config = lib.mkIf cfg.enable {
    boot = {
      kernelParams = ["acpi_enforce_resources=lax"];
      kernelModules = [
        "nct6775"
        "it87"
        "k10temp"
      ];
      extraModprobeConfig = ''
        options it87 ignore_resource_conflict=1 force_id=${cfg.forceId}
      '';
    };
  };
}
