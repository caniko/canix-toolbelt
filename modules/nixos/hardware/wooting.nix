{
  config,
  lib,
  pkgs,
  ...
}: {
  options.canix-toolbelt.hardware.wooting.enable = lib.mkEnableOption "Wooting keyboard support";

  config = lib.mkIf config.canix-toolbelt.hardware.wooting.enable {
    services.udev.packages = [pkgs.wooting-udev-rules];
  };
}
