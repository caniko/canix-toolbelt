{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.services.wol-relay;
  allHosts = config.canix-toolbelt.hosts;

  # Filter hosts that have macAddress and lanBroadcast defined.
  wolCapableHosts = lib.filterAttrs (_: h: h.macAddress != null && h.lanBroadcast != null) allHosts;

  wolScript = name: hostData:
    pkgs.writeShellScript "wol-${name}" ''
      echo "Sending Wake-on-LAN packet to ${name} (${hostData.macAddress})"
      ${pkgs.wol}/bin/wol -i ${hostData.lanBroadcast} ${hostData.macAddress}
    '';

  makeWolService = name: hostData: {
    "wol-${name}" = {
      description = "Wake-on-LAN for ${name}";
      serviceConfig = {
        Type = "oneshot";
        ExecStart = wolScript name hostData;
      };
    };
  };
in {
  options.canix-toolbelt.services.wol-relay = {
    enable = lib.mkEnableOption "Wake-on-LAN relay service";
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [pkgs.wol];

    systemd.services = lib.mkMerge (lib.mapAttrsToList makeWolService wolCapableHosts);
  };
}
