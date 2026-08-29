{
  config,
  lib,
  options,
  ...
}: let
  nm = import ../../../lib/networkmanager.nix {inherit lib;};
  hostname = config.networking.hostName;
  hostData = config.canix-toolbelt.hosts.${hostname} or {};
  wakeOnLanInterface = hostData.wakeOnLanInterface or null;
  hasFacterDhcpInterfaces =
    options ? facter
    && options.facter ? detected
    && options.facter.detected ? dhcp
    && options.facter.detected.dhcp ? interfaces;
in {
  imports = [
    ../registry/hosts.nix
  ];

  config = lib.mkIf config.networking.networkmanager.enable (lib.mkMerge [
    {
      systemd.services.NetworkManager-wait-online.enable = false;
      systemd.services.systemd-networkd-wait-online.enable = false;
    }
    (lib.mkIf hasFacterDhcpInterfaces {
      facter.detected.dhcp.interfaces = [];
    })
    (lib.mkIf (wakeOnLanInterface != null) {
      networking.networkmanager.ensureProfiles.profiles.${wakeOnLanInterface} =
        nm.mkAutoWakeOnLanProfile wakeOnLanInterface;
    })
  ]);
}
