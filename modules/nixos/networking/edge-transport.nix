{
  config,
  lib,
  ...
}: let
  inherit (lib) mkOption types;
  net = import ../../../lib/network-types.nix {inherit lib;};
  cfg = config.canix-toolbelt.networking.edgeTransport;
  peers = builtins.attrValues cfg.peers;
in {
  options.canix-toolbelt.networking.edgeTransport = {
    enable = lib.mkEnableOption "a dedicated, peer-restricted WireGuard edge transport";
    interface = mkOption {
      type = net.interfaceName;
      default = "wg-edge";
      description = "Dedicated transport interface, independent of any forwarded VPN service.";
    };
    address = mkOption {
      type = net.ipv4;
      description = "Local IPv4 tunnel address. Each peer receives an exact /32 route.";
    };
    privateKeyFile = mkOption {
      type = net.runtimeKeyFile;
      description = "Runtime secret or persistent /var/lib key path; never a Nix-store private key.";
    };
    listenPort = mkOption {
      type = types.port;
      description = "Local WireGuard UDP port, distinct from forwarded UDP listeners.";
    };
    mtu = mkOption {
      type = types.ints.between 1280 9000;
      description = "Measured tunnel MTU. Reserve WireGuard and outer-IP overhead from the underlay MTU.";
    };
    openFirewall = mkOption {
      type = types.bool;
      default = false;
      description = "Open the local WireGuard port. Enable on the edge; home can initiate through NAT.";
    };
    peers = mkOption {
      default = {};
      type = types.attrsOf (types.submodule {
        options = {
          address = mkOption {
            type = net.ipv4;
            description = "Peer's exact tunnel IPv4 address.";
          };
          publicKey = mkOption {
            type = types.strMatching "[A-Za-z0-9+/]{43}=";
            description = "Enrolled WireGuard public key.";
          };
          endpoint = mkOption {
            type = types.nullOr (types.strMatching "[^[:space:]]+:[0-9]+");
            default = null;
            description = "Independent underlay endpoint host:port, set on the home-initiated side.";
          };
          keepaliveSeconds = mkOption {
            type = types.nullOr (types.ints.between 1 65535);
            default = null;
            description = "Persistent keepalive for an initiating peer behind NAT (typically 25).";
          };
        };
      });
      description = "Enrolled transport peers; arbitrary client/default-route prefixes are not accepted.";
    };
  };
  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion =
          peers
          != []
          && builtins.length (lib.unique (map (p: p.address) peers)) == builtins.length peers
          && !(builtins.elem cfg.address (map (p: p.address) peers));
        message = "edgeTransport: declare unique peer addresses distinct from the local address.";
      }
      {
        assertion = builtins.all (p: p.endpoint == null || p.keepaliveSeconds != null) peers;
        message = "edgeTransport: home-initiated endpoints require persistent keepalive.";
      }
      {
        assertion =
          cfg.interface
          != "wg-home"
          && builtins.length (lib.unique (map (p: p.publicKey) peers)) == builtins.length peers;
        message = "edgeTransport: use independent wg-edge identity and unique peer public keys.";
      }
    ];
    networking.wireguard.interfaces.${cfg.interface} = {
      ips = ["${cfg.address}/32"];
      inherit (cfg) privateKeyFile listenPort mtu;
      peers =
        map (peer: {
          inherit (peer) publicKey endpoint;
          allowedIPs = ["${peer.address}/32"];
          persistentKeepalive = peer.keepaliveSeconds;
        })
        peers;
    };
    networking.firewall.allowedUDPPorts = lib.optional cfg.openFirewall cfg.listenPort;
  };
}
