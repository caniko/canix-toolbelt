# Offline fixture addresses and upstream's public snakeoil key, never enrollment data.
{
  pkgs,
  lib,
  ...
}: let
  keys = import "${pkgs.path}/nixos/tests/wireguard/snakeoil-keys.nix";
in {
  canix-toolbelt.networking.edgeTransport = {
    enable = true;
    address = "10.77.0.1";
    privateKeyFile = "/var/lib/wireguard/edge.key";
    listenPort = 51821;
    mtu = 1380;
    openFirewall = true;
    peers.home = {
      address = "10.77.0.2";
      publicKey = lib.trim keys.peer1.publicKey;
    };
  };
  canix-toolbelt.services.publicEdge = {
    enable = true;
    publicAddress = "192.0.2.10";
    publicInterface = "ens3";
    http."app.example.test".upstreams = [
      {
        address = "10.77.0.2";
        port = 443;
      }
    ];
    tcp = {
      git = {
        publicPort = 22;
        address = "10.77.0.2";
        port = 22;
      };
      mail = {
        publicPort = 25;
        address = "10.77.0.2";
        port = 25;
        proxyProtocol = "v1";
      };
    };
    udp = {
      webtransport = {
        publicPort = 8443;
        address = "10.77.0.2";
        port = 8443;
      };
      vpn = {
        publicPort = 54321;
        address = "10.77.0.2";
        port = 54321;
      };
    };
  };
}
