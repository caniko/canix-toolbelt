{pkgs}: let
  inherit (pkgs) lib;
  python = pkgs.python3.withPackages (p: [p.aioquic p.aiohttp]);
  keys = import "${pkgs.path}/nixos/tests/wireguard/snakeoil-keys.nix";
  # Public, disposable test keys/certificates only. Real private keys are runtime
  # secrets and never generated in or copied from the Nix store.
  pki = pkgs.runCommand "public-edge-test-pki" {nativeBuildInputs = [pkgs.openssl];} ''
    mkdir -p "$out"
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
      -keyout ca.key -out "$out/ca.pem" -days 3650 -subj '/CN=Public edge test CA' \
      -addext 'basicConstraints=critical,CA:TRUE' -addext 'keyUsage=critical,keyCertSign,cRLSign'
    for name in app.example.test origin.example.test wrong.example.test; do
      openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
        -keyout "$out/$name.key" -out request.pem -subj "/CN=$name" \
        -addext "subjectAltName=DNS:$name" -addext 'basicConstraints=critical,CA:FALSE' \
        -addext 'keyUsage=critical,digitalSignature' -addext 'extendedKeyUsage=serverAuth'
      openssl x509 -req -in request.pem -CA "$out/ca.pem" -CAkey ca.key \
        -CAcreateserial -out "$out/$name.pem" -days 3650 -copy_extensions copy
    done
  '';
  nm = import ../lib/networkmanager.nix {inherit lib;};
  common = address: {
    networking = {
      useDHCP = false;
      dhcpcd.enable = false;
      networkmanager = {
        enable = true;
        settings.main.no-auto-default = "*";
        ensureProfiles.profiles.test-uplink = nm.mkEthernetProfile {
          id = "test-uplink";
          interfaceName = "eth1";
          ipv4 = {
            method = "manual";
            address1 = "${address}/24";
          };
          ipv6.method = "disabled";
        };
      };
      firewall.enable = true;
      nftables.enable = true;
      hosts."192.0.2.1" = ["app.example.test"];
    };
    virtualisation = {
      memorySize = 768;
      # Suppress both address families from the test harness; preconfigured
      # IPv6 makes NM assume an external connection instead of this profile.
      interfaces.eth1.vlan = 1;
    };
    environment.systemPackages = [pkgs.curl pkgs.jq pkgs.wireguard-tools pkgs.conntrack-tools pkgs.iproute2 python];
    security.pki.certificateFiles = ["${pki}/ca.pem"];
  };
  certificates = names: {
    systemd.tmpfiles.rules =
      ["d /var/lib/edge-test-pki 0750 caddy caddy - -"]
      ++ lib.concatMap (name: [
        "C /var/lib/edge-test-pki/${name}.pem 0640 caddy caddy - ${pki}/${name}.pem"
        "C /var/lib/edge-test-pki/${name}.key 0640 caddy caddy - ${pki}/${name}.key"
      ])
      names;
    canix-toolbelt.services.caddy.certificates =
      map (name: {
        certificate = "/var/lib/edge-test-pki/${name}.pem";
        key = "/var/lib/edge-test-pki/${name}.key";
      })
      names;
  };
  transport = edge: {
    canix-toolbelt.networking.edgeTransport = {
      enable = true;
      address =
        if edge
        then "10.77.0.1"
        else "10.77.0.2";
      privateKeyFile = "/run/wg-fixture.key";
      listenPort =
        if edge
        then 51821
        else 51822;
      mtu = 1380;
      openFirewall = edge;
      peers.remote = {
        address =
          if edge
          then "10.77.0.2"
          else "10.77.0.1";
        publicKey = lib.trim (
          if edge
          then keys.peer1.publicKey
          else keys.peer0.publicKey
        );
        endpoint =
          if edge
          then null
          else "192.0.2.1:51821";
        keepaliveSeconds =
          if edge
          then null
          else 1;
      };
    };
    systemd.tmpfiles.rules = [
      "f+ /run/wg-fixture.key 0600 root root - ${
        if edge
        then keys.peer0.privateKey
        else keys.peer1.privateKey
      }"
    ];
  };
  reverse = dial: {
    handler = "reverse_proxy";
    upstreams = [{inherit dial;}];
  };
in
  pkgs.testers.runNixOSTest {
    name = "public-edge";
    meta.timeout = 600;
    globalTimeout = 600;
    nodes = {
      edge = {
        imports = [../modules/nixos/services/public-edge.nix (common "192.0.2.1") (transport true) (certificates ["app.example.test"])];
        canix-toolbelt.services.publicEdge = {
          enable = true;
          publicAddress = "192.0.2.1";
          publicInterface = "eth1";
          http."app.example.test" = {
            upstreams = [
              {
                address = "10.77.0.2";
                port = 443;
              }
            ];
            serverName = "origin.example.test";
            caBundle = "${pki}/ca.pem";
          };
          tcp = {
            git = {
              publicPort = 22;
              address = "10.77.0.2";
              port = 2222;
            };
            proxy = {
              publicPort = 25;
              address = "10.77.0.2";
              port = 2525;
              proxyProtocol = "v1";
            };
          };
          udp.webtransport = {
            publicPort = 8443;
            address = "10.77.0.2";
            port = 8443;
          };
        };
      };
      origin = {
        imports = [
          ../modules/nixos/networking/edge-transport.nix
          ../modules/nixos/services/caddy-base.nix
          (common "192.0.2.2")
          (transport false)
          (certificates ["app.example.test" "origin.example.test"])
        ];
        networking.firewall.interfaces.wg-edge = {
          allowedTCPPorts = [443 2222 2525];
          allowedUDPPorts = [8443];
        };
        canix-toolbelt.services.caddy = {
          enable = true;
          servers = {
            ingress = {
              listen = ["10.77.0.2:443"];
              protocols = ["h1" "h2"];
              cidrAllowlist = ["10.77.0.1/32"];
              trustedProxies = ["10.77.0.1/32"];
              routes = [
                {
                  match = [{host = ["app.example.test"];}];
                  handle = [(reverse "127.0.0.1:8081")];
                }
              ];
            };
            relay = {
              listen = ["127.0.0.1:8081"];
              automaticHttps = false;
              cidrAllowlist = ["127.0.0.1/32"];
              # Source admission is immediate-hop only; strict XFF parsing also
              # needs the known earlier proxy hop in the sanitized chain.
              trustedProxies = ["127.0.0.1/32" "10.77.0.1/32"];
              routes = [
                {
                  match = [{host = ["app.example.test"];}];
                  handle = [(reverse "127.0.0.1:8082" // {headers.request.set."X-Fixture-Client" = ["{http.vars.client_ip}"];})];
                }
              ];
            };
          };
        };
        systemd.services = {
          caddy = {
            after = ["wireguard-wg-edge.service"];
            requires = ["wireguard-wg-edge.service"];
          };
          edge-application = {
            wantedBy = ["multi-user.target"];
            after = ["wireguard-wg-edge.service"];
            requires = ["wireguard-wg-edge.service"];
            serviceConfig.ExecStart = "${python}/bin/python ${./fixtures/edge-application.py}";
          };
          edge-webtransport = {
            wantedBy = ["multi-user.target"];
            after = ["wireguard-wg-edge.service"];
            requires = ["wireguard-wg-edge.service"];
            serviceConfig.ExecStart = "${python}/bin/python ${./fixtures/edge-webtransport.py} server --host 10.77.0.2 --certificate ${pki}/app.example.test.pem --key ${pki}/app.example.test.key";
          };
        };
      };
      client.imports = [(common "192.0.2.3")];
    };
    testScript = ''
      import hashlib
      import json

      start_all()
      for host in [edge, origin, client]:
          host.wait_for_unit("NetworkManager.service")
          try:
              host.wait_until_succeeds("ip -o -4 addr show dev eth1 | grep -q '192.0.2.'", timeout=30)
          except Exception:
              print(host.succeed("nmcli device; nmcli connection; journalctl -b -u NetworkManager --no-pager"))
              raise
      edge.wait_for_unit("wireguard-wg-edge.service")
      origin.wait_for_unit("edge-application.service")
      origin.wait_for_unit("edge-webtransport.service")
      edge.wait_for_unit("caddy.service")
      origin.wait_for_unit("caddy.service")
      edge.wait_for_unit("haproxy.service")
      origin.wait_until_succeeds("ping -c 1 -W 1 10.77.0.1")
      assert "0.0.0.0/0" not in edge.succeed("wg show wg-edge allowed-ips")
      assert "wg-edge" not in client.succeed("ip route get 192.0.2.2")

      curl = "curl --fail --silent --show-error --max-time 20 https://app.example.test"
      client.wait_until_succeeds(curl)
      for flag, version in [("--http1.1", "1.1"), ("--http2", "2"), ("--http3-only", "3")]:
          assert client.succeed(f"{curl} {flag} -o /dev/null -w '%{{http_version}}'").strip() == version

      spoofed = client.succeed(curl + " --http3-only -H 'X-Forwarded-For: 203.0.113.66' -H 'X-Real-IP: 203.0.113.66' -H 'Forwarded: for=203.0.113.66' -H 'CF-Connecting-IP: 203.0.113.66' -H 'X-Forwarded-Proto: http' -H 'X-Forwarded-Host: attacker.test'")
      headers = {k.lower(): v for k, v in json.loads(spoofed).items()}
      assert headers["host"] == "app.example.test", headers
      assert headers["x-fixture-client"] == "192.0.2.3", headers
      assert headers["x-real-ip"] == "192.0.2.3", headers
      assert headers["x-forwarded-proto"] == "https", headers
      assert "203.0.113.66" not in str(headers), headers
      assert "forwarded" not in headers and "cf-connecting-ip" not in headers, headers

      client.succeed("head -c 8388608 /dev/zero > /tmp/upload")
      expected = hashlib.sha256(bytes(8388608)).hexdigest()
      assert client.succeed(curl + "/upload --http3-only --data-binary @/tmp/upload").strip() == expected
      expected = hashlib.sha256(b"x" * 8388608).hexdigest()
      assert client.succeed(curl + "/download --http3-only | sha256sum").split()[0] == expected
      client.succeed("python -c 'import asyncio, aiohttp; exec(\"async def test():\\n async with aiohttp.ClientSession() as c:\\n  async with c.ws_connect(\\\"wss://app.example.test/ws\\\") as ws:\\n   await ws.send_str(\\\"websocket-roundtrip\\\")\\n   assert (await ws.receive()).data == \\\"websocket-roundtrip\\\"\\nasyncio.run(test())\")'")

      # Drop QUIC while preserving the TLS/TCP listener: curl must fall back.
      client.succeed("nft add table ip h3_block; nft 'add chain ip h3_block output { type filter hook output priority filter; }'; nft add rule ip h3_block output ip daddr 192.0.2.1 udp dport 443 drop")
      assert client.succeed(curl + " --http3 -o /dev/null -w '%{http_version}'").strip() == "2"
      client.succeed("nft delete table ip h3_block")

      client.succeed("python -c 'import socket; s=socket.create_connection((\"192.0.2.1\",22),5); assert s.recv(256).startswith(b\"SSH-2.0-edge-fixture\")'")
      client.succeed("python -c 'import socket; s=socket.create_connection((\"192.0.2.1\",25),5); h=s.recv(256); assert h.startswith(b\"PROXY TCP4 192.0.2.3 192.0.2.1 \"), h'")

      wt = "python ${./fixtures/edge-webtransport.py} client --host 192.0.2.1 --ca ${pki}/ca.pem"
      # Two sessions from the same public address exercise independent NAT state.
      client.succeed(f"{wt} > /tmp/wt-one.log & first=$!; {wt} > /tmp/wt-two.log & second=$!; wait $first; first_status=$?; wait $second; second_status=$?; test $first_status = 0 && test $second_status = 0")
      client.succeed(wt + " --rebind")
      client.succeed(wt)  # clean new session after closing the previous connection

      # Deterministic path bounds plus real packet loss and latency.
      edge.succeed("ip link set eth1 mtu 1360; ip link set wg-edge mtu 1280; tc qdisc add dev eth1 root netem delay 5ms loss 1%")
      origin.succeed("ip link set eth1 mtu 1360; ip link set wg-edge mtu 1280")
      client.succeed(wt)
      edge.succeed("tc qdisc del dev eth1 root")

      # Undeclared ports and directly routed origin requests must fail.
      client.fail("timeout 5 " + wt + " --port 8444")
      client.succeed("ip route add 10.77.0.2/32 via 192.0.2.1")
      client.fail("curl --fail --max-time 3 --resolve app.example.test:443:10.77.0.2 https://app.example.test")
      client.fail("curl --fail --max-time 3 --resolve app.example.test:443:192.0.2.2 https://app.example.test")

      # A wrong origin certificate must fail closed despite an intact tunnel.
      for suffix in ["pem", "key"]:
          origin.succeed(f"cp ${pki}/wrong.example.test.{suffix} /var/lib/edge-test-pki/origin.example.test.{suffix}")
      origin.succeed("systemctl restart caddy")
      client.fail(curl)
      for suffix in ["pem", "key"]:
          origin.succeed(f"cp ${pki}/origin.example.test.{suffix} /var/lib/edge-test-pki/origin.example.test.{suffix}")
      origin.succeed("systemctl restart caddy")
      client.wait_until_succeeds(curl)

      # No public-origin fallback when the dedicated transport is down.
      edge.succeed("systemctl stop wireguard-wg-edge.service")
      client.fail(curl)
      edge.succeed("systemctl start wireguard-wg-edge.service")
      client.wait_until_succeeds(curl)
      client.succeed(wt)
    '';
  }
