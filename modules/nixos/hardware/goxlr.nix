{
  config,
  lib,
  pkgs,
  ...
}: {
  options.canix-toolbelt.hardware.goxlr.enable = lib.mkEnableOption "GoXLR audio mixer support";

  config = lib.mkIf config.canix-toolbelt.hardware.goxlr.enable {
    environment.systemPackages = [pkgs.goxlr-utility];
    services.goxlr-utility.enable = true;
  };
}
