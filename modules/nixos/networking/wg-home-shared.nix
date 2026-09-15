{
  config,
  lib,
  ...
}: let
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

  # When the wg-home link is declared (via fleetix integration), derive wgHome
  # defaults only from explicit link fields. Consumers must set vpnDomain and
  # endpointHost themselves (canix does in root/modules/networking/wg_home.nix);
  # no fleet domain is implied here.
  config.canix-toolbelt.networking.wgHome = lib.mkIf (wgLink.cidr != null) (
    lib.mkMerge [
      (lib.mkIf ((wgLink.vpnDomain or null) != null) {
        vpnDomain = lib.mkDefault wgLink.vpnDomain;
      })
      (lib.mkIf ((wgLink.endpointHost or wgLink.ddnsHost or wgLink.endpointSubdomain or null) != null) {
        endpointHost = lib.mkDefault (wgLink.endpointHost or wgLink.ddnsHost or wgLink.endpointSubdomain);
      })
      {
        port = lib.mkDefault (wgLink.port or 54321);
      }
    ]
  );
}
