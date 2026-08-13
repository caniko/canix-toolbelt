{pkgs}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  evaluate = wrappedApps:
    (import "${pkgs.path}/nixos/lib/eval-config.nix" {
      system = "x86_64-linux";
      modules = [
        ../modules/nixos/networking/vpn-netns.nix
        {
          system.stateVersion = "25.11";

          canix-toolbelt.networking.vpnNetns = {
            enable = true;
            privateKeyFile = "/test/wg-key";
            ips = ["10.14.0.2/16"];
            peers = [
              {
                publicKey = "uRT3uSAUwvm7Rp+s3n5V9JibsKHKAZ/8+SU3psG8QxI=";
                endpoint = "ro-buc.prod.surfshark.com:51820";
              }
            ];
            dnsServers = ["162.252.172.57"];
            boundServices = ["dummy"];
            portForwards = {qbittorrent = 8085;};
            socks = true;
            inherit wrappedApps;
          };

          systemd.services.dummy = {
            wantedBy = ["multi-user.target"];
            serviceConfig.ExecStart = "${pkgs.coreutils}/bin/sleep infinity";
          };
        }
      ];
    }).config;

  plain = evaluate {};
  withApps = evaluate {
    qbittorrent = {
      package = pkgs.qbittorrent;
      icon = "qbittorrent";
    };
  };

  wg = plain.networking.wireguard.interfaces.wg0;
  setupScript = plain.systemd.services.netns-vpn-setup.serviceConfig.ExecStart;
  qbitFwd = plain.systemd.services."netns-vpn-forward-qbittorrent".serviceConfig.ExecStart;
  socksFwdExists = plain.systemd.services ? "netns-vpn-forward-microsocks";
in
  mkEvalCheck {
    name = "vpn-netns-eval";
    resultMessage = "vpn-netns module wires netns, wireguard, forwards and wrapped apps";
    runtimeScript = ''
      grep -q '^#!' ${setupScript}
    '';
    assertions = [
      {
        name = "wireguard-in-namespace";
        assertion = wg.interfaceNamespace == "vpn";
        message = "wg interface must be created inside the vpn namespace";
      }
      {
        name = "wireguard-key-and-addresses";
        assertion = wg.privateKeyFile == "/test/wg-key" && wg.ips == ["10.14.0.2/16"];
        message = "key file and addresses must flow into the wireguard config";
      }
      {
        name = "netns-creation-unit";
        assertion = builtins.elem "wireguard-wg0.service" plain.systemd.services.netns-vpn.requiredBy;
        message = "netns unit must be pulled in by the wireguard unit";
      }
      {
        name = "setup-unit-wanted";
        assertion = plain.systemd.services.netns-vpn-setup.wantedBy == ["multi-user.target"];
        message = "setup unit must be enabled at boot";
      }
      {
        name = "bound-service-namespace";
        assertion =
          plain.systemd.services.dummy.serviceConfig.NetworkNamespacePath
          == "/run/netns/vpn"
          && plain.systemd.services.dummy.serviceConfig.BindReadOnlyPaths == ["/etc/netns/vpn/resolv.conf:/etc/resolv.conf"];
        message = "bound services must move into the namespace and get its resolv.conf";
      }
      {
        name = "port-forward-unit";
        assertion =
          lib.hasInfix "/run/netns/vpn" (toString qbitFwd)
          && lib.hasInfix "8085" (toString qbitFwd);
        message = "socat forward must target the namespace";
      }
      {
        name = "socks-proxy-unit";
        assertion =
          plain.systemd.services."netns-vpn-sockets".wantedBy
          == ["multi-user.target"]
          && socksFwdExists;
        message = "socks proxy and its host forward must exist";
      }
      {
        name = "namespace-resolv-conf";
        assertion = plain.environment.etc."netns/vpn/resolv.conf".text == "nameserver 162.252.172.57";
        message = "resolv.conf must be staged for the namespace";
      }
      {
        name = "wrapped-app-desktop-entry";
        assertion = lib.any (p: builtins.match ".*qbittorrent.*" p.name != null) withApps.environment.systemPackages;
        message = "wrapped apps must get a generated desktop entry package";
      }
      {
        name = "wrapped-app-vpn-exec";
        assertion = lib.any (p: p.name == "vpn-exec") withApps.environment.systemPackages;
        message = "vpn-exec launcher must be installed";
      }
      {
        name = "sudo-nopasswd-rule";
        assertion = lib.any (r:
          r.groups
          == ["wheel"]
          && lib.any (c: lib.hasInfix "vpn-exec" c.command) r.commands)
        withApps.security.sudo.extraRules;
        message = "sudo NOPASSWD rule must exist for vpn-exec";
      }
    ];
  }
