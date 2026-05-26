{
  config,
  lib,
  ...
}: {
  options.canix-toolbelt.hardware.openrgb.enable = lib.mkEnableOption "OpenRGB lighting control";

  config = lib.mkIf config.canix-toolbelt.hardware.openrgb.enable {
    services.hardware.openrgb.enable = true;
  };
}
