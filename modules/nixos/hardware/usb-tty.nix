{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.hardware.usbTty;
in {
  options.canix-toolbelt.hardware.usbTty = {
    enable = lib.mkEnableOption "USB TTY device access";

    user = lib.mkOption {
      type = lib.types.str;
      default = "can";
      description = "User to grant USB TTY device access.";
    };
  };

  config = lib.mkIf cfg.enable {
    users.groups.dialout = {};
    users.users.${cfg.user}.extraGroups = ["dialout"];

    services.udev.extraRules = ''
      KERNEL=="ttyUSB[0-9]*", GROUP="dialout", MODE="0660", TAG+="uaccess"
      KERNEL=="ttyACM[0-9]*", GROUP="dialout", MODE="0660", TAG+="uaccess"
    '';
  };
}
