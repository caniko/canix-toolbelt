{lib}: rec {
  inherit (lib) optionalAttrs;

  constants = {
    tunMode.tap = "2";
    wakeOnLan.magic = "64";
  };

  mkEthernetProfile = {
    id,
    interfaceName ? null,
    autoconnect ? true,
    connection ? {},
    ethernet ? {},
    ipv4 ? {method = "auto";},
    ipv6 ? {method = "auto";},
  }: {
    connection =
      {
        inherit id;
        type = "ethernet";
        autoconnect =
          if autoconnect
          then "true"
          else "false";
      }
      // optionalAttrs (interfaceName != null) {
        interface-name = interfaceName;
      }
      // connection;
    inherit ethernet ipv4 ipv6;
  };

  mkSharedEthernetProfile = args @ {
    addresses,
    sharedDhcpRange ? null,
    ...
  }:
    mkEthernetProfile (
      builtins.removeAttrs args ["addresses" "sharedDhcpRange"]
      // {
        ipv4 =
          {
            method = "shared";
            inherit addresses;
          }
          // optionalAttrs (sharedDhcpRange != null) {
            shared-dhcp-range = sharedDhcpRange;
          };
        ipv6.method = "disabled";
      }
    );

  mkAutoWakeOnLanProfile = interfaceName:
    mkEthernetProfile {
      id = interfaceName;
      inherit interfaceName;
      ethernet.wake-on-lan = constants.wakeOnLan.magic;
    };
}
