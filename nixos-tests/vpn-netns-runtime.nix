{pkgs}: let
  keys = import "${pkgs.path}/nixos/tests/wireguard/snakeoil-keys.nix";
  legacyContent = pkgs.writeTextDir "index.html" "legacy namespace\n";
  mediaContent = pkgs.writeTextDir "index.html" "media namespace\n";
in
  pkgs.testers.runNixOSTest {
    name = "vpn-netns-runtime";

    nodes.machine = {
      imports = [../modules/nixos/networking/vpn-netns.nix];
      environment.systemPackages = [pkgs.curl pkgs.iproute2];
      environment.etc."test-wg-key".text = keys.peer0.privateKey;

      canix-toolbelt.networking = {
        vpnNetns = {
          enable = true;
          privateKeyFile = "/etc/test-wg-key";
          ips = ["10.14.0.2/16"];
          dnsServers = ["192.0.2.53"];
          peers = [
            {
              publicKey = keys.peer1.publicKey;
              allowedIPs = ["0.0.0.0/0"];
            }
          ];
          boundServices = ["legacy-http"];
          portForwards.http = 18081;
        };
        vpnNamespaces.media = {
          profile = {
            provider = "test";
            dnsServers = ["192.0.2.53"];
            connection = {
              type = "wireguard";
              addresses = ["10.15.0.2/16"];
              privateKeyRef = "test-wg-key";
              peers = [
                {
                  publicKey = keys.peer1.publicKey;
                  endpoint = "192.0.2.1:51820";
                  allowedIps = ["0.0.0.0/0"];
                }
              ];
            };
          };
          privateKeyFile = "/etc/test-wg-key";
          boundServices = ["media-http"];
          portForwards.http = 18082;
        };
      };

      systemd.services = {
        legacy-http = {
          wantedBy = ["multi-user.target"];
          serviceConfig.ExecStart = "${pkgs.python3}/bin/python3 -m http.server 18081 --bind 127.0.0.1 --directory ${legacyContent}";
        };
        media-http = {
          wantedBy = ["multi-user.target"];
          serviceConfig.ExecStart = "${pkgs.python3}/bin/python3 -m http.server 18082 --bind 127.0.0.1 --directory ${mediaContent}";
        };
      };
    };

    testScript = ''
      start_all()
      for unit in (
          "legacy-http", "media-http",
          "netns-vpn-forward-http", "netns-vpn-media-forward-http",
      ):
          machine.wait_for_unit(f"{unit}.service")

      for namespace, port, content in (
          ("vpn", 18081, "legacy namespace"),
          ("vpn-media", 18082, "media namespace"),
      ):
          url = f"http://127.0.0.1:{port}/"
          machine.wait_until_succeeds(
              f"ip netns exec {namespace} curl --fail --silent --max-time 2 {url} | grep -Fx '{content}'"
          )
          # Exercise the systemd-rendered ExecStart and socat's address parser,
          # not just the presence of a port or a namespace in its command text.
          machine.wait_until_succeeds(
              f"curl --fail --silent --max-time 2 {url} | grep -Fx '{content}'"
          )
          for _ in range(3):
              assert machine.succeed(f"curl --fail --silent --max-time 2 {url}").strip() == content

      machine.succeed("ss -ltnH 'sport = :18082' | grep -F '127.0.0.1:18082'")
      machine.succeed("systemctl restart netns-vpn-media-forward-http.service")
      machine.wait_until_succeeds(
          "curl --fail --silent --max-time 2 http://127.0.0.1:18082/ | grep -Fx 'media namespace'"
      )
    '';
  }
