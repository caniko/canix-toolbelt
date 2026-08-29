# Direct P2P ethernet link between two hosts.
#
# A "gateway" host serves NAT (NM shared mode) on its directLinkInterface; a
# "client" host connects with a static IP and routes through the gateway.
# Topology data (IPs, interface, MAC) lives in canix-toolbelt.hosts on each host.
# NetworkManager leaves the profile inactive when the cable or peer is absent.
{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.networking.directLink;
  hostname = config.networking.hostName;
  nm = import ../../../lib/networkmanager.nix {inherit lib;};

  selfHost = config.canix-toolbelt.hosts.${hostname} or null;
  topologyRole =
    if selfHost != null
    then selfHost.directLinkRole or null
    else null;
  effectiveRole =
    if cfg.role != null
    then cfg.role
    else topologyRole;
  gatewayHost =
    if cfg.gateway != null
    then config.canix-toolbelt.hosts.${cfg.gateway}
    else null;
in {
  imports = [
    ../registry/hosts.nix
  ];

  options.canix-toolbelt.networking.directLink = {
    enable = lib.mkEnableOption "Direct P2P ethernet link via NetworkManager";

    role = lib.mkOption {
      type = lib.types.nullOr (lib.types.enum ["gateway" "client"]);
      default = null;
      description = ''
        gateway: serve NAT (NM "shared" mode) on directLinkInterface.
        client:  static IP, default route via gateway's directLinkIp.
      '';
    };

    gateway = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "On client role, the hostname of the gateway host (looked up in canix-toolbelt.hosts).";
    };

    mtu = lib.mkOption {
      type = lib.types.str;
      default = "9000";
      description = "Ethernet MTU for the direct link (jumbo frames by default).";
    };

    sharedDhcpRange = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "10.0.0.2,10.0.0.254";
      description = "On gateway role, optional DHCP range NM hands out on the shared link.";
    };
  };

  config = lib.mkIf cfg.enable (lib.mkMerge [
    {
      assertions = [
        {
          assertion = selfHost != null && selfHost.directLinkIp != null;
          message = "canix-toolbelt.networking.directLink requires ${hostname} to have network.directLinkIp in Fleetix topology";
        }
        {
          assertion = effectiveRole != null;
          message = "canix-toolbelt.networking.directLink requires role = \"gateway\"/\"client\" or a Fleetix direct-link binding role";
        }
        {
          assertion = effectiveRole != "client" || cfg.gateway != null;
          message = "canix-toolbelt.networking.directLink.gateway must be set when role = \"client\"";
        }
        {
          assertion = effectiveRole != "gateway" || selfHost.directLinkInterface != null;
          message = "canix-toolbelt.networking.directLink: role = \"gateway\" requires ${hostname} to have network.directLinkInterface in Fleetix topology";
        }
      ];
    }

    (lib.mkIf (effectiveRole == "gateway") {
      networking.networkmanager.ensureProfiles.profiles.direct-link = nm.mkSharedEthernetProfile ({
          id = "direct-link";
          interfaceName = selfHost.directLinkInterface;
          ethernet.mtu = cfg.mtu;
          addresses = "${selfHost.directLinkIp}/24";
        }
        // lib.optionalAttrs (cfg.sharedDhcpRange != null) {
          inherit (cfg) sharedDhcpRange;
        });

      # NM's shared-mode dnsmasq listens on directLinkIp:53 for the client.
      # Without this the NixOS firewall drops DNS queries from the client.
      networking.firewall.interfaces.${selfHost.directLinkInterface} = {
        allowedTCPPorts = [53];
        allowedUDPPorts = [53];
      };

      # NetworkManager launches dnsmasq for shared profiles.  The packaged
      # unit historically used KillMode=process, which can orphan dnsmasq on
      # a NetworkManager restart and leave 10.10.0.1:53 permanently occupied.
      # Kill the helper children with the gateway manager instead.
      systemd.services.NetworkManager.serviceConfig.KillMode = lib.mkForce "mixed";
    })

    (lib.mkIf (effectiveRole == "client") {
      networking.networkmanager.ensureProfiles.profiles.direct-link = {
        connection = {
          id = "direct-link";
          type = "ethernet";
          autoconnect = "true";
        };
        ethernet =
          {inherit (cfg) mtu;}
          // lib.optionalAttrs (selfHost.directLinkMac != null) {
            mac-address = selfHost.directLinkMac;
          };
        ipv4 = {
          method = "manual";
          addresses = "${selfHost.directLinkIp}/24";
          gateway = gatewayHost.directLinkIp;
          dns = gatewayHost.directLinkIp;
          route-metric = "100";
        };
        ipv6.method = "disabled";
      };
    })
  ]);
}
