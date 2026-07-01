{pkgs}: let
  lib = pkgs.lib;
  fleetix = (import ../lib {inherit lib;}).fleetix;
  topology = {
    links = {
      wg-home = {
        __pkl_class = "Link";
        subnet = "10.1.0.0/24";
        port = 51820;
        endpointSubdomain = "wg";
      };
      otg.subnet = "10.55.0.0/24";
    };
    hosts = {
      server = {
        __pkl_class = "Host";
        deviceType = "server";
        network.lanIp = "192.0.2.1";
        links.wg-home = {
          address = "10.1.0.1";
          role = "server";
          publicKey = "server-key";
        };
      };
      client = {
        __pkl_class = "Host";
        deviceType = "laptop";
        network.lanIp = "192.0.2.2";
        links = {
          wg-home = {
            address = "10.1.0.2";
            publicKey = "client-key";
          };
          otg = {
            address = "10.55.0.2";
            externalInterface = "usb0";
          };
        };
      };
    };
    domains = {
      __pkl_class = "Domains";
      zones = ["example.com" "example.net" "example.org"];
      mailSubdomain = "mail";
      vpnSubdomain = "vpn";
    };
    services = {
      __pkl_class = "Services";
      sshPort = 2222;
      reverseProxyServices = [
        {
          __pkl_class = "ReverseProxyService";
          name = "demo-service";
          hostname = "demo.example.com";
          port = 8080;
          targetHost = "server";
        }
      ];
      emailIdentities = {
        __pkl_class = "EmailIdentities";
        adminEmail = "admin@example.com";
      };
    };
  };
  normalized = fleetix.normalizeAll {
    inherit topology;
    serviceHostAliases = {
      demoService = "demo-service";
      legacyDemo = "legacy.example.com";
    };
    directLinkNames = ["direct-link" "otg"];
  };
  assertions = [
    (normalized.hosts.client.network.wgHomeIp == "10.1.0.2")
    (normalized.hosts.client.network.directLinkInterface == "usb0")
    (normalized.links.wg-home.serverAddress == "10.1.0.1")
    (normalized.links.wg-home.endpointHost == "wg-home.example.net")
    (normalized.links.wg-home.ddnsHost == "wg-home.example.net")
    (normalized.links.wg-home.peers
      == [
        {
          name = "client";
          publicKey = "client-key";
          allowedIPs = ["10.1.0.2/32"];
        }
      ])
    (normalized.domains.serviceHosts.demoService == "demo.example.com")
    (normalized.domains.serviceHosts.legacyDemo == "legacy.example.com")
    (normalized.domains.wgEndpointHost == "wg-home.example.net")
    (normalized.services.emailIdentities == {adminEmail = "admin@example.com";})
    (normalized.services.reverseProxyByName.demo-service.targetHost == "server")
  ];
in
  pkgs.runCommand "fleetix-lib-eval" {} ''
    echo ${lib.escapeShellArg (builtins.toJSON assertions)} > "$TMPDIR/assertions.json"
    if grep -q false "$TMPDIR/assertions.json"; then
      cat "$TMPDIR/assertions.json" >&2
      exit 1
    fi
    touch "$out"
  ''
