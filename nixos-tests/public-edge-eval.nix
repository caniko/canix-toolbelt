{
  inputs,
  pkgs,
}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  evaluate = modules:
    import "${pkgs.path}/nixos/lib/eval-config.nix" {
      system = pkgs.stdenv.hostPlatform.system;
      modules =
        [
          inputs.disko.nixosModules.disko
          ../modules/nixos/hosts/cloud-host.nix
          ../modules/nixos/services/public-edge.nix
          ./fixtures/cloud-host.nix
          ./fixtures/public-edge.nix
          {canix-toolbelt.cloudHost.access.port = 1337;}
        ]
        ++ modules;
    };
  edge = evaluate [];
  server = edge.config.canix-toolbelt.services.caddy.servers.public-edge;
  proxy = builtins.elemAt (builtins.head server.routes).handle 1;
  disabled = import "${pkgs.path}/nixos/lib/eval-config.nix" {
    system = pkgs.stdenv.hostPlatform.system;
    modules = [../modules/nixos/services/public-edge.nix];
  };
  invalid = module: message:
    builtins.any (a: !a.assertion && lib.hasPrefix message a.message) (evaluate [module]).config.assertions;
  badKeyPath = evaluate [{canix-toolbelt.networking.edgeTransport.privateKeyFile = lib.mkForce "/nix/store/private-key";}];
in
  mkEvalCheck {
    name = "public-edge-eval";
    assertions = [
      {
        name = "complete-edge-host";
        assertion = builtins.all (a: a.assertion) edge.config.assertions && builtins.isString edge.config.system.build.toplevel.drvPath;
        message = "The optional edge role must compose with the complete cloud host";
      }
      {
        name = "independent-protocols";
        assertion =
          server.protocols
          == ["h1" "h2" "h3"]
          && !server.allow0Rtt
          && proxy.transport.versions == ["1.1" "2"]
          && proxy.transport.tls.server_name == "app.example.test"
          && proxy.upstreams == [{dial = "10.77.0.2:443";}]
          && builtins.elem 443 edge.config.networking.firewall.interfaces.ens3.allowedUDPPorts;
        message = "HTTP/3 ingress needs UDP 443 and independently verified origin TLS/protocols";
      }
      {
        name = "peer-only-routes";
        assertion =
          (builtins.head edge.config.networking.wireguard.interfaces.wg-edge.peers).allowedIPs
          == ["10.77.0.2/32"]
          && !(edge.config.networking.wireguard.interfaces ? wg-home);
        message = "The outer transport must not take over the forwarded VPN or install a default route";
      }
      {
        name = "disabled-is-inert";
        assertion =
          !disabled.config.services.caddy.enable
          && !disabled.config.services.haproxy.enable
          && disabled.config.networking.wireguard.interfaces == {}
          && !(disabled.config.networking.nftables.tables ? public-edge);
        message = "Importing the public role must leave a plain guest inert";
      }
      {
        name = "reject-ssh-collision";
        assertion = invalid {canix-toolbelt.cloudHost.access.port = lib.mkForce 22;} "publicEdge: public listener ports";
        message = "Git and administrative SSH cannot bind the same port";
      }
      {
        name = "reject-quic-and-tunnel-collisions";
        assertion =
          invalid {canix-toolbelt.services.publicEdge.udp.webtransport.publicPort = lib.mkForce 443;} "publicEdge: public listener ports"
          && invalid {canix-toolbelt.networking.edgeTransport.listenPort = lib.mkForce 54321;} "publicEdge: public listener ports";
        message = "Application QUIC, HTTP/3 and the outer/inner VPN ports must remain distinct";
      }
      {
        name = "reject-unenrolled-origin";
        assertion = invalid {canix-toolbelt.services.publicEdge.udp.webtransport.address = lib.mkForce "10.77.0.99";} "publicEdge: every origin";
        message = "Forwarding must fail before deployment when no enrolled WireGuard peer owns the destination";
      }
      {
        name = "reject-mixed-origin-h3";
        assertion = invalid {canix-toolbelt.services.publicEdge.http."app.example.test".originProtocols = ["2" "3"];} "publicEdge: origin HTTP/3";
        message = "Do not render unsupported Caddy HTTP/3 transport combinations";
      }
      {
        name = "reject-store-private-key";
        assertion = !(builtins.tryEval (builtins.deepSeq badKeyPath.config.canix-toolbelt.networking.edgeTransport.privateKeyFile true)).success;
        message = "The transport requires a runtime secret path";
      }
      {
        name = "require-nat-keepalive";
        assertion = invalid {canix-toolbelt.networking.edgeTransport.peers.home.endpoint = "192.0.2.11:51821";} "edgeTransport: home-initiated endpoints";
        message = "The initiating peer must maintain its NAT mapping";
      }
    ];
    nativeBuildInputs = [pkgs.jq pkgs.caddy pkgs.haproxy];
    runtimeScript = ''
      jq -e '.apps.http.servers["public-edge"] | .protocols == ["h1","h2","h3"] and .allow_0rtt == false and .trusted_proxies == null' ${edge.config.services.caddy.configFile} >/dev/null
      # Provision in the build sandbox with a local test CA and sandbox log paths.
      jq 'del(.logging) | .apps.tls.automation.policies = [{"issuers":[{"module":"internal"}]}]' ${edge.config.services.caddy.configFile} > caddy.json
      export HOME="$TMPDIR"
      caddy validate --config caddy.json
      haproxy -c -f ${pkgs.writeText "edge-haproxy.cfg" edge.config.services.haproxy.config}
    '';
  }
