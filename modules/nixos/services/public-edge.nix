{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.services.publicEdge;
  tunnel = config.canix-toolbelt.networking.edgeTransport;
  net = import ../../../lib/network-types.nix {inherit lib;};
  httpPorts = lib.optionals (cfg.http != {}) [80 443];
  tcpPorts = map (f: f.publicPort) (builtins.attrValues cfg.tcp);
  udpPorts = map (f: f.publicPort) (builtins.attrValues cfg.udp);
  unique = values: builtins.length (lib.unique values) == builtins.length values;
  destinations =
    lib.concatMap (site: site.upstreams) (builtins.attrValues cfg.http)
    ++ builtins.attrValues cfg.tcp ++ builtins.attrValues cfg.udp;
  peers = map (peer: peer.address) (builtins.attrValues tunnel.peers);
in {
  imports = [./public-edge-options.nix ./public-edge-forwarding.nix ./caddy-base.nix ../networking/edge-transport.nix];
  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = tunnel.enable && cfg.publicInterface != tunnel.interface;
        message = "publicEdge: enable a dedicated edgeTransport distinct from the public uplink.";
      }
      {
        assertion = builtins.all (d: builtins.elem d.address peers) destinations;
        message = "publicEdge: every origin must be an enrolled edgeTransport peer address.";
      }
      {
        assertion =
          unique (tcpPorts ++ httpPorts ++ lib.optionals config.services.openssh.enable config.services.openssh.ports)
          && unique (udpPorts ++ lib.optional (cfg.http != {}) 443 ++ [tunnel.listenPort]);
        message = "publicEdge: public listener ports must be unique and distinct from administrative SSH and WireGuard.";
      }
      {
        assertion =
          builtins.all net.hostname.check (builtins.attrNames cfg.http)
          && builtins.all (n: builtins.match "[a-zA-Z0-9_-]+" n != null) (builtins.attrNames cfg.tcp ++ builtins.attrNames cfg.udp);
        message = "publicEdge: HTTP names must be hostnames and forwarding names must be identifier-safe.";
      }
      {
        assertion =
          builtins.all (site: !(builtins.elem "3" site.originProtocols) || site.originProtocols == ["3"])
          (builtins.attrValues cfg.http);
        message = "publicEdge: origin HTTP/3 must be configured independently as originProtocols = [\"3\"].";
      }
    ];

    canix-toolbelt.services.caddy = lib.mkIf (cfg.http != {}) {
      enable = true;
      servers.public-edge = {
        listen = ["${cfg.publicAddress}:443"];
        protocols = ["h1" "h2" "h3"];
        allow0Rtt = false;
        routes =
          lib.mapAttrsToList (hostname: site: {
            match = [{host = [hostname];}];
            handle = [
              {
                handler = "headers";
                request = {
                  delete = ["Forwarded" "X-Forwarded-*" "CF-Connecting-IP" "CF-Connecting-IPv6" "True-Client-IP"];
                  # HeaderOps deletes after setting: replace X-Real-IP without
                  # including it in delete, so the canonical value survives.
                  set."X-Real-IP" = ["{http.request.remote.host}"];
                };
              }
              {
                handler = "reverse_proxy";
                upstreams = map (upstream: {dial = "${upstream.address}:${toString upstream.port}";}) site.upstreams;
                transport = {
                  protocol = "http";
                  versions = site.originProtocols;
                  tls =
                    {server_name = site.serverName;}
                    // lib.optionalAttrs (site.caBundle != null) {
                      ca = {
                        provider = "file";
                        pem_files = [site.caBundle];
                      };
                    };
                };
                # Caddy 2.11 otherwise rewrites Host to upstream TLS ServerName.
                headers.request.set.Host = ["{http.request.hostport}"];
                health_checks.active = {
                  uri = site.healthPath;
                  headers.Host = [hostname];
                  expect_status = 200;
                  interval = "10s";
                  timeout = "3s";
                };
              }
            ];
            terminal = true;
          })
          cfg.http;
      };
    };
    networking.firewall = {
      enable = true;
      interfaces.${cfg.publicInterface} = {
        allowedTCPPorts = httpPorts ++ tcpPorts;
        allowedUDPPorts = lib.optional (cfg.http != {}) 443;
      };
    };
    networking.nftables.enable = true;
    systemd.services = {
      caddy = lib.mkIf (cfg.http != {}) {
        wants = ["network-online.target"];
        after = ["network-online.target"];
      };
      haproxy = lib.mkIf (cfg.tcp != {}) {
        wants = ["network-online.target"];
        after = ["network-online.target"];
      };
    };
  };
}
