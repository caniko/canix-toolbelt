{config, lib, ...}: let
  inherit (lib) mkOption types;
  wgLink = config.canix-toolbelt.networking.links.wg-home or {};
in {
  options.canix-toolbelt.networking = {
    wgHome = {
      vpnDomain = mkOption {
        type = types.str;
        description = "VPN DNS domain served over wg-home.";
      };

      endpointHost = mkOption {
        type = types.str;
        description = "Public DNS hostname wg-home clients dial.";
      };

      port = mkOption {
        type = types.port;
        description = "WireGuard UDP port used by wg-home.";
      };
    };

  };

  # When the wg-home link is declared (via fleetix integration), derive wgHome defaults
  config.canix-toolbelt.networking.wgHome = lib.mkIf (wgLink.cidr != null) {
    vpnDomain = lib.mkDefault "vpn.${wgLink.endpointSubdomain or "wg"}.candee.baby";
    endpointHost = lib.mkDefault (wgLink.endpointSubdomain or "wg");
    port = lib.mkDefault (wgLink.port or 54321);
  };
}
