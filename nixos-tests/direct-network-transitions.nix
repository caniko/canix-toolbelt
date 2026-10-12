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
    mkdir -p "$out/bin" "$out/tmp" "$out/nix/store" "$out/run/direct-network" "$out/run/systemd/journal" "$out/run/host" "$out/dev" "$out/proc" "$out/sys"
    # Only the fixture's shell/utility closure is in the image. The readiness
    # client and its live socket must be supplied by the enrolled module.
    while IFS= read -r path; do
      cp -a "$path" "$out/nix/store/"
    done < ${(pkgs.closureInfo {rootPaths = [pkgs.bash pkgs.coreutils];})}/store-paths
    # Immutable roots must already contain bind mount points. Keep additional
    # readiness contents absent so their read-only binds are still exercised.
    while IFS= read -r path; do
      if [ ! -e "$out$path" ]; then
        if [ -d "$path" ]; then
          mkdir -p "$out$path"
        else
          touch "$out$path"
        fi
      fi
    done < ${(pkgs.closureInfo {rootPaths = [package];})}/store-paths
    ln -s ${pkgs.bash}/bin/bash "$out/bin/sh"
    touch "$out/inside-root"
  '';
  serviceImage = pkgs.runCommand "direct-network-service-root.ext4" {nativeBuildInputs = [pkgs.e2fsprogs];} ''
    truncate -s 256M "$out"
    mkfs.ext4 -q -F -d ${serviceRoot} "$out"
  '';
  rootedService = {
    Type = "oneshot";
    User = "nobody";
    MemoryMax = "64M";
    BindPaths = ["/run/direct-network-root-state:/tmp"];
    # All original hooks retain the configured root and UID. No blanket store
    # bind may hide a missing readiness-client runtime dependency.
    ExecStartPre = ["/bin/sh -c 'test -e /inside-root && test -S /run/direct-network/ready.sock && test \"$(${pkgs.coreutils}/bin/id -u)\" = 65534 && echo pre > /tmp/%N-pre'"];
    ExecStart = "/bin/sh -c 'test -e /inside-root && test \"$(${pkgs.coreutils}/bin/id -u)\" = 65534 && echo payload > /tmp/%N-payload'";
    ExecStartPost = "/bin/sh -c 'test -e /inside-root && test \"$(${pkgs.coreutils}/bin/id -u)\" = 65534 && echo post > /tmp/%N-post'";
  };
in
  pkgs.testers.runNixOSTest {
    name = "direct-network-transitions";
    nodes.machine = {...}: {
      imports = [inputs.home-manager.nixosModules.home-manager ../modules/nixos/networking/direct-network.nix];
      virtualisation.memorySize = 1536;
      # The enrolled workload's explicit retry policy must remain authoritative.
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
          services.rooted-user-start = "app-amc.slice";
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
        # Admission must survive the resets and precede the effective hook.
        ExecStartPre = ["${pkgs.coreutils}/bin/false" "" "${pkgs.coreutils}/bin/false" "" "${pkgs.coreutils}/bin/touch /tmp/direct-network-late-system-pre"];
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
          ExecStart = "/bin/sh -c 'test -e /inside-root && test \"$(${pkgs.coreutils}/bin/id -u)\" = 65534 && echo payload > /tmp/%N-payload && exec ${pkgs.coreutils}/bin/sleep infinity'";
        };
      systemd.services.rooted-retry.unitConfig.StartLimitIntervalSec = 0;
      home-manager.users.operator.systemd.user.services.late-start.Service = {
        Type = "oneshot";
        ExecStartPre = ["${pkgs.coreutils}/bin/false" "" "${pkgs.coreutils}/bin/false" "" "${pkgs.coreutils}/bin/touch /home/operator/direct-network-late-user-pre"];
        ExecStart = "${pkgs.coreutils}/bin/touch /home/operator/direct-network-late-user";
      };
      home-manager.users.operator.systemd.user.services.rooted-user-start.Service =
        rootedService
        // {
          RootDirectory = serviceRoot;
          WorkingDirectory = "/";
          User = "operator";
          # Exercise a real user-manager namespace without the readiness client
          # in the root. All authored hooks retain the same root and user.
          ExecStartPre = ["/bin/sh -c 'exit 1'" "" "/bin/sh -c 'test -e /inside-root && test -S /run/direct-network/ready.sock && test \"$(${pkgs.coreutils}/bin/id -u)\" = 1000 && echo pre > /tmp/%N-pre'"];
          ExecStart = "/bin/sh -c 'test -e /inside-root && test \"$(${pkgs.coreutils}/bin/id -u)\" = 1000 && echo payload > /tmp/%N-payload'";
          ExecStartPost = "/bin/sh -c 'test -e /inside-root && test \"$(${pkgs.coreutils}/bin/id -u)\" = 1000 && echo post > /tmp/%N-post'";
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
