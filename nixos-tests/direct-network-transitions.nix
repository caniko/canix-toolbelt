{
  pkgs,
  inputs,
}: let
  fixture = ./direct-network-fixture.py;
  python = "${pkgs.python3}/bin/python3";
  package = pkgs.callPackage ../nix/package.nix {directNetwork = true;};
in
  pkgs.testers.runNixOSTest {
    name = "direct-network-transitions";
    nodes.machine = {...}: {
      imports = [inputs.home-manager.nixosModules.home-manager ../modules/nixos/networking/direct-network.nix];
      virtualisation.memorySize = 1536;
      networking = {
        useDHCP = false;
        firewall.enable = false;
        networkmanager.enable = true;
        networkmanager.unmanaged = ["interface-name:uplink" "interface-name:outer" "interface-name:wg-home" "interface-name:wg-proton" "interface-name:pvpnksintrf0"];
      };
      services.resolved = {
        enable = true;
        settings.Resolve = {
          DNS = "198.18.0.2";
          Cache = false;
        };
      };
      users.users.operator = {
        isNormalUser = true;
        uid = 1000;
        linger = true;
      };
      environment.systemPackages = [pkgs.python3 pkgs.iproute2 pkgs.wireguard-tools pkgs.curl pkgs.dnsutils package];
      canix-toolbelt.networking.directNetwork = {
        enable = true;
        inherit package;
        directInterfaces = ["uplink" "wg-home"];
        networkManagerDns = false;
        dnsServers = ["198.18.0.2"];
        dnsZones."vpn.example.test" = "10.123.0.1";
        users.operator = {
          services.backend = "app-amc.slice";
          slices = ["agent-tools.slice"];
        };
        systemServices.backup = "canix-background.slice";
      };
      home-manager.users.operator = {
        home.stateVersion = "24.11";
        systemd.user.slices.app-amc.Slice.MemoryMax = "512M";
        systemd.user.slices.agent-tools.Slice.MemoryMax = "256M";
        systemd.user.services.backend = {
          Unit.Description = "Persistent connections established before host VPN changes";
          Service = {
            ExecStart = "${python} ${fixture} client /home/operator/state.json";
            MemoryMax = "128M";
          };
        };
      };
      systemd.slices.canix-background.sliceConfig.MemoryMax = "512M";
      systemd.services.backup.serviceConfig = {
        Type = "oneshot";
        User = "operator";
        MemoryMax = "128M";
        ExecStart = "${python} ${fixture} probe /home/operator/system-probe.json";
      };
      systemd.timers.backup.timerConfig.OnActiveSec = "1s";
      systemd.services.network-fixture.serviceConfig.ExecStart = "${python} ${fixture} setup ${pkgs.dnsmasq}/bin/dnsmasq";
      # Keep this real listener in the protected service cgroup to verify replies
      # to desktop clients, rather than depending on namespace port forwarding.
      home-manager.users.operator.systemd.user.services.listener.Service.ExecStart = "${python} -m http.server 4096 --bind 127.0.0.1";
      canix-toolbelt.networking.directNetwork.users.operator.services.listener = "app.slice";
    };
    testScript = ''
        import json

        start_all()
        machine.wait_for_unit("multi-user.target")
        machine.succeed("systemctl start network-fixture.service")
        machine.wait_for_file("/run/network-fixture-ready")
        machine.wait_for_unit("user@1000.service")
        machine.wait_until_succeeds("canix-toolbelt-direct-network ready")
        user = "runuser -u operator -- env XDG_RUNTIME_DIR=/run/user/1000 "
        userctl = user + "systemctl --user "
      userrun = user + "systemd-run --user --quiet --wait --pipe "
        probe = "${python} ${fixture} probe"
        machine.succeed(userctl + "start backend.service listener.service")
        machine.wait_for_file("/home/operator/state.json")
        before = json.loads(machine.succeed("cat /home/operator/state.json"))["counter"]
        pid = machine.succeed(userctl + "show backend.service -p MainPID --value").strip()

        def check_direct():
            machine.wait_until_succeeds(userctl + "is-active backend.service")
            result = json.loads(machine.succeed(userrun + "--slice=agent-tools.slice " + probe))
            assert result["v4"] == result["v6"] == result["fleet"] == "direct", result
            assert result["dns"] == ["2001:db8:20::20", "203.0.113.20"], result
            assert result["fleetDns"] == "10.123.0.1", result
            machine.succeed("curl --fail --max-time 5 http://127.0.0.1:4096/")
            assert machine.succeed(userctl + "show backend.service -p MainPID --value").strip() == pid
            assert machine.succeed(userctl + "show backend.service -p NRestarts --value").strip() == "0"

        check_direct()
        for family in ["-4", "-6"]:
            machine.succeed(f"ip {family} route add default dev wg-proton table 51820")
            machine.succeed(f"ip {family} rule add priority 32764 lookup main suppress_prefixlength 0")
            machine.succeed(f"ip {family} rule add priority 32765 not fwmark 51820 lookup 51820")
        machine.succeed("resolvectl dns wg-proton 172.31.0.2")
        machine.succeed("resolvectl domain wg-proton '~.'")
        machine.succeed("resolvectl flush-caches")
        desktop = json.loads(machine.succeed(userrun + "--slice=app.slice " + probe))
        assert desktop["v4"] == desktop["v6"] == "vpn", desktop
        assert desktop["fleet"] == "direct", desktop
        assert desktop["dns"] == ["198.51.100.20", "2001:db8:30::20"], desktop
        check_direct()

        # A system-manager timer running as the same UID remains direct too.
        machine.succeed("systemctl start backup.timer")
        machine.wait_for_file("/home/operator/system-probe.json")
        system = json.loads(machine.succeed("cat /home/operator/system-probe.json"))
        assert system["v4"] == system["v6"] == "direct", system
        machine.succeed("ip netns exec vpnpeer wg set wg-proton listen-port 51821")
        machine.succeed("wg set wg-proton peer $(cat /run/wg-proton-peer.pub) endpoint 198.19.0.2:51821")
        check_direct()
        machine.succeed("curl -4 --fail --max-time 5 http://203.0.113.20:8080/ | grep vpn")

        # Lost tunnel + advanced-kill-switch-shaped IPv4/IPv6 dummy routes/DNS.
        machine.succeed("ip link set wg-proton down")
        for family in ["-4", "-6"]:
            machine.succeed(f"ip {family} route flush table 51820")
        machine.succeed("ip link add pvpnksintrf0 type dummy")
        machine.succeed("ip address add 100.85.0.1/24 dev pvpnksintrf0")
        machine.succeed("ip -6 address add fd85::1/64 dev pvpnksintrf0 nodad")
        machine.succeed("ip link set pvpnksintrf0 up")
        for family in ["-4", "-6"]:
            machine.succeed(f"ip {family} route add default dev pvpnksintrf0 metric 1")
        machine.succeed("resolvectl revert wg-proton")
        machine.succeed("resolvectl dns pvpnksintrf0 100.85.0.2")
        machine.succeed("resolvectl domain pvpnksintrf0 '~.'")
        machine.fail("curl --fail --max-time 2 http://203.0.113.20:8080/")
        check_direct()

        machine.succeed("ip link del pvpnksintrf0")
        machine.succeed("ip link set wg-proton up")
        for family in ["-4", "-6"]:
            machine.succeed(f"ip {family} route add default dev wg-proton table 51820")
        machine.succeed("resolvectl dns wg-proton 172.31.0.2")
        machine.succeed("resolvectl domain wg-proton '~.'")
        machine.succeed("resolvectl flush-caches")
        check_direct()
        machine.succeed("curl --fail --max-time 5 http://203.0.113.20:8080/ | grep vpn")
        for family in ["-4", "-6"]:
            machine.succeed(f"ip {family} rule del priority 32764")
            machine.succeed(f"ip {family} rule del priority 32765")
        machine.succeed("resolvectl revert wg-proton")
        check_direct()
        machine.succeed("curl --fail --max-time 5 http://203.0.113.20:8080/ | grep direct")
        machine.wait_until_succeeds("${python} -c 'import json; assert json.load(open(\"/home/operator/state.json\"))[\"counter\"] > " + str(before + 3) + "'")
        assert machine.succeed(userctl + "show app-amc.slice -p MemoryMax --value").strip() == "536870912"
        assert machine.succeed(userctl + "show backend.service -p MemoryMax --value").strip() == "134217728"
    '';
  }
