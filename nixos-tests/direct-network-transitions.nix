{
  pkgs,
  inputs,
}: let
  fixture = ./direct-network-fixture.py;
  python = "${pkgs.python3}/bin/python3";
  negative = ./direct-network-negative.py;
  systemctlFixture = pkgs.writeScript "direct-network-systemctl-fixture" ("#!${python}\n" + builtins.replaceStrings ["@SYSTEMCTL@"] ["${pkgs.systemd}/bin/systemctl"] (builtins.readFile ./direct-network-systemctl.py));
  nmcliFixture = pkgs.writeScript "direct-network-nmcli-fixture" ("#!${python}\n" + builtins.readFile ./direct-network-nmcli.py);
  package = pkgs.callPackage ../nix/package.nix {directNetwork = true;};
  serviceRoot = pkgs.runCommand "direct-network-service-root" {} ''
    mkdir -p "$out/bin" "$out/tmp" "$out/nix/store" "$out/run"
    ln -s ${pkgs.bash}/bin/bash "$out/bin/sh"
    touch "$out/inside-root"
  '';
  serviceImage = pkgs.runCommand "direct-network-service-root.ext4" {nativeBuildInputs = [pkgs.e2fsprogs];} ''
    truncate -s 16M "$out"
    mkfs.ext4 -q -F -d ${serviceRoot} "$out"
  '';
  rootedService = {
    Type = "oneshot";
    User = "nobody";
    MemoryMax = "64M";
    BindReadOnlyPaths = ["/nix/store"];
    BindPaths = ["/run/direct-network-root-state:/tmp"];
    # Both commands must retain their alternate root and the readiness socket
    # deliberately remains unavailable inside it.
    ExecStartPre = ["/bin/sh -c 'test -e /inside-root && test ! -S /run/direct-network/ready.sock && echo pre > /tmp/%N-pre'"];
    ExecStart = "/bin/sh -c 'test -e /inside-root && echo payload > /tmp/%N-payload'";
  };
in
  pkgs.testers.runNixOSTest {
    name = "direct-network-transitions";
    nodes.machine = {...}: {
      imports = [inputs.home-manager.nixosModules.home-manager ../modules/nixos/networking/direct-network.nix];
      virtualisation.memorySize = 1536;
      # Keep the default burst of five, with a window long enough that slow
      # hosted execution cannot clear the helper's counter during this test.
      systemd.settings.Manager = {
        DefaultStartLimitIntervalSec = "60s";
        DefaultStartLimitBurst = 5;
      };
      # The test driver's default VLAN uses 2001:db8:1::/64 too. This fixture
      # owns all provider/fleet networks and must not use the QEMU WAN route.
      virtualisation.vlans = [];
      networking = {
        useDHCP = false;
        firewall = {
          enable = true;
          allowedTCPPorts = [4096];
          allowedUDPPorts = [51818 51819];
        };
        networkmanager.enable = true;
        networkmanager.unmanaged = ["interface-name:eth0" "interface-name:uplink" "interface-name:outer" "interface-name:wg-home" "interface-name:wg-proton" "interface-name:pvpnksintrf0" "interface-name:ecmp-status" "interface-name:ecmp-peer"];
      };
      services.resolved = {
        enable = true;
        settings.Resolve = {
          DNS = "198.18.0.2";
          Cache = false;
          DNSStubListenerExtra = "[::1]:53";
        };
      };
      users.users.operator = {
        isNormalUser = true;
        uid = 1000;
        linger = true;
      };
      environment.systemPackages = [pkgs.python3 pkgs.iproute2 pkgs.wireguard-tools pkgs.curl pkgs.dnsutils pkgs.nftables pkgs.conntrack-tools package];
      canix-toolbelt.networking.directNetwork = {
        enable = true;
        inherit package;
        directInterfaces = ["uplink" "wg-home" "ecmp-status"];
        networkManagerDns = false;
        dnsServers = ["198.18.0.2"];
        dnsZones."vpn.example.test" = "10.123.0.1";
        users.operator = {
          services.backend = "app-amc.slice";
          services.late-start = "app-amc.slice";
          slices = ["agent-tools.slice"];
        };
        systemServices.backup = "canix-background.slice";
        systemServices.late-start = "canix-background.slice";
        systemServices.rooted-start = "canix-background.slice";
        systemServices.imaged-start = "canix-background.slice";
        systemServices.rooted-retry = "canix-background.slice";
      };
      home-manager.users.operator = {
        home.stateVersion = "24.11";
        systemd.user.slices.app-amc.Slice.MemoryMax = "512M";
        systemd.user.slices.agent-tools.Slice.MemoryMax = "256M";
        systemd.user.services.backend = {
          Unit.Description = "Persistent connections established before host VPN changes";
          Service = {
            ExecStart = "${python} ${fixture} client /home/operator/state.json";
            ExecStartPre = "${pkgs.coreutils}/bin/true";
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
      # Nobody has no readiness UID enrollment. Its narrow privileged pre-start
      # client must gate the payload without widening normal user authorization.
      systemd.services.late-start.serviceConfig = {
        Type = "oneshot";
        User = "nobody";
        ExecStart = "${pkgs.coreutils}/bin/touch /tmp/direct-network-late-system";
      };
      systemd.tmpfiles.rules = ["d /run/direct-network-root-state 0777 root root -"];
      systemd.services.rooted-start.serviceConfig = rootedService // {RootDirectory = serviceRoot;};
      systemd.services.imaged-start.serviceConfig = rootedService // {RootImage = serviceImage;};
      systemd.services.rooted-retry.serviceConfig =
        rootedService
        // {
          Type = "exec";
          RootDirectory = serviceRoot;
          Restart = "always";
          RestartSec = "100ms";
          ExecStart = "/bin/sh -c 'test -e /inside-root && echo payload > /tmp/%N-payload && exec ${pkgs.coreutils}/bin/sleep infinity'";
        };
      systemd.services.rooted-retry.unitConfig.StartLimitIntervalSec = 0;
      home-manager.users.operator.systemd.user.services.late-start.Service = {
        Type = "oneshot";
        ExecStart = "${pkgs.coreutils}/bin/touch /home/operator/direct-network-late-user";
      };
      systemd.services.network-fixture = {
        path = [pkgs.iproute2 pkgs.wireguard-tools];
        serviceConfig.ExecStart = "${python} ${fixture} setup ${pkgs.dnsmasq}/bin/dnsmasq";
      };
      # Keep this real listener in the protected service cgroup to verify replies
      # to desktop clients, rather than depending on namespace port forwarding.
      home-manager.users.operator.systemd.user.services.listener.Service.ExecStart = "${python} ${fixture} server listener 4096";
      canix-toolbelt.networking.directNetwork.users.operator.services.listener = "app.slice";
    };
    testScript = builtins.replaceStrings ["@PYTHON@" "@FIXTURE@" "@NEGATIVE@" "@SYSTEMCTL_FIXTURE@" "@NMCLI_FIXTURE@"] [python "${fixture}" "${negative}" "${systemctlFixture}" "${nmcliFixture}"] (builtins.readFile ./direct-network-test.py);
  }
