{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (config.networking) hostName;
  host = config.canix-toolbelt.hosts.${hostName} or {};
in {
  imports = [../registry/hosts.nix];

  config = lib.mkIf ((host.deviceType or null) == "desktop") {
    environment.systemPackages = [pkgs.vial];
    services.udev.packages = [pkgs.vial];
  };
}
