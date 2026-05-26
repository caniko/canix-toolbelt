{
  config,
  lib,
  options,
  ...
}: let
  cfg = config.canix-toolbelt.networking.networkmanager;
  nm = import ../../../lib/networkmanager.nix;
  hasFacterDhcpInterfaces =
    options ? facter
    && options.facter ? detected
    && options.facter.detected ? dhcp
    && options.facter.detected.dhcp ? interfaces;
in {
  options.canix-toolbelt.networking.networkmanager.wakeOnLanInterface = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = null;
    description = ''
      Interface name that should receive a NetworkManager profile enabling
      Wake-on-LAN magic packets.
    '';
  };

  config = lib.mkIf config.networking.networkmanager.enable (lib.mkMerge [
    {
      systemd.services.NetworkManager-wait-online.enable = false;
      systemd.services.systemd-networkd-wait-online.enable = false;
    }
    (lib.mkIf hasFacterDhcpInterfaces {
      facter.detected.dhcp.interfaces = [];
    })
    (lib.mkIf (cfg.wakeOnLanInterface != null) {
      networking.networkmanager.ensureProfiles.profiles.${cfg.wakeOnLanInterface} =
        nm.mkAutoWakeOnLanProfile cfg.wakeOnLanInterface;
    })
  ]);
}
