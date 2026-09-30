{lib, ...}: let
  inherit (lib) mkOption types;
  net = import ../../../lib/network-types.nix {inherit lib;};
  destination = {
    address = mkOption {
      type = net.ipv4;
      description = "Enrolled origin's WireGuard IPv4 address; never its public service DNS name.";
    };
    port = mkOption {
      type = types.port;
      description = "Destination listener port on the origin.";
    };
  };
  forwarding = protocol:
    types.submodule {
      options =
        destination
        // {
          publicPort = mkOption {
            type = types.port;
            description = "Public ${protocol} port to forward.";
          };
        }
        // lib.optionalAttrs (protocol == "TCP") {
          proxyProtocol = mkOption {
            type = types.enum ["none" "v1"];
            default = "none";
            description = "PROXY v1 for a separately configured trusted listener (e.g. Stalwart); none for Git SSH.";
          };
        };
    };
in {
  options.canix-toolbelt.services.publicEdge = {
    enable = lib.mkEnableOption "provider-neutral public ingress over the dedicated edge transport";
    publicAddress = mkOption {
      type = net.ipv4;
      description = "IPv4 address assigned to the public interface. IPv6 publication needs a separately validated path.";
    };
    publicInterface = mkOption {
      type = net.interfaceName;
      description = "Public uplink interface used to scope forwarding.";
    };
    http = mkOption {
      default = {};
      description = "Published HTTP hostnames and their tunnel-bound HTTPS origins.";
      type = types.attrsOf (types.submodule ({name, ...}: {
        options = {
          serverName = mkOption {
            type = net.hostname;
            default = name;
            defaultText = lib.literalExpression "name";
            description = "Origin TLS SNI and certificate name; the public HTTP Host is preserved separately.";
          };
          upstreams = mkOption {
            type = types.nonEmptyListOf (types.submodule {options = destination;});
            description = "Tunnel-bound HTTPS origins. No automatic retry of a partially sent request.";
          };
          originProtocols = mkOption {
            type = types.nonEmptyListOf (types.enum ["1.1" "2" "3"]);
            default = ["1.1" "2"];
            description = "Origin-facing HTTP versions, independent of public HTTP/3. Caddy requires 3 to be exclusive.";
          };
          caBundle = mkOption {
            type = types.nullOr types.path;
            default = null;
            description = "Optional PEM CA bundle for origin TLS; null uses system trust. Verification is always enabled.";
          };
          healthPath = mkOption {
            type = types.strMatching "/[^[:space:]]*";
            default = "/";
            description = "GET path returning 200 when this Host/SNI/backend is healthy.";
          };
        };
      }));
    };
    tcp = mkOption {
      type = types.attrsOf (forwarding "TCP");
      default = {};
      description = "Named HAProxy TCP listeners, with TLS retained at the application.";
    };
    udp = mkOption {
      type = types.attrsOf (forwarding "UDP");
      default = {};
      description = "Named DNAT/SNAT listeners, including application-owned QUIC/WebTransport and forwarded VPNs.";
    };
  };
}
