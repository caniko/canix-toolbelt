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
        firewall = {
          enable = true;
          allowedTCPPorts = [4096];
          allowedUDPPorts = [51818 51819];
        };
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
      home-manager.users.operator.systemd.user.services.listener.Service.ExecStart = "${python} ${fixture} server listener 4096";
      canix-toolbelt.networking.directNetwork.users.operator.services.listener = "app.slice";
    };
    testScript = builtins.replaceStrings ["@PYTHON@" "@FIXTURE@"] [python "${fixture}"] (builtins.readFile ./direct-network-test.py);
  }
