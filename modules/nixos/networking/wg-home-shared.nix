{lib, ...}: let
  inherit (lib) mkOption types;
in {
  options.canix-toolbelt.networking.wgHome = {
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
}
