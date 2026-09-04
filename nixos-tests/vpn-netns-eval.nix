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

  plural =
    (import "${pkgs.path}/nixos/lib/eval-config.nix" {
      system = "x86_64-linux";
      modules = [
        ../modules/nixos/networking/vpn-netns.nix
        {
          system.stateVersion = "25.11";
          canix-toolbelt.networking.vpnNamespaces = {
            can = {
              profile = {
                provider = "Example VPN";
                owner = "can";
                dnsServers = ["10.2.0.1"];
                connection = {
                  type = "wireguard";
                  addresses = ["10.2.0.2/32"];
                  privateKeyRef = "vpn/can/private-key";
                  peers = [
                    {
                      publicKey = "can-peer-key";
                      endpoint = "can.example.test:51820";
                      allowedIps = ["0.0.0.0/0"];
                      dynamicEndpointRefreshSeconds = 30;
                      dynamicEndpointRefreshRestartSeconds = 5;
                    }
                  ];
                };
                portForwarding = {
                  type = "nat-pmp";
                  gateway = "10.2.0.1";
                };
              };
              privateKeyFile = "/test/can-key";
              boundServices = ["can-service"];
              portForwards.qbittorrent = 8085;
              socks.enable = true;
              wrappedApps.qbittorrent = {
                bin = "${pkgs.coreutils}/bin/true";
                allowedUsers = ["can"];
              };
              renewal.command = "${pkgs.coreutils}/bin/true";
            };
            dejana = {
              profile = {
                provider = "Example VPN";
                owner = "dejana";
                dnsServers = ["10.3.0.1"];
                connection = {
                  type = "wireguard";
                  addresses = ["10.3.0.2/32"];
                  privateKeyRef = "vpn/dejana/private-key";
                  peers = [
                    {
                      publicKey = "dejana-peer-key";
                      endpoint = "dejana.example.test:51820";
                      allowedIps = ["0.0.0.0/0"];
                    }
                  ];
                };
              };
              privateKeyFile = "/test/dejana-key";
              boundServices = ["dejana-service"];
              portForwards.qbittorrent = 8086;
            };
          };
          systemd.services = {
            can-service.serviceConfig.ExecStart = "${pkgs.coreutils}/bin/true";
            dejana-service.serviceConfig.ExecStart = "${pkgs.coreutils}/bin/true";
          };
        }
      ];
    }).config;

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
      {
        name = "plural-wireguard-namespaces";
        assertion =
          plural.networking.wireguard.interfaces.wg-can.interfaceNamespace
          == "vpn-can"
          && plural.networking.wireguard.interfaces.wg-dejana.interfaceNamespace == "vpn-dejana"
          && plural.systemd.services.can-service.serviceConfig.NetworkNamespacePath == "/run/netns/vpn-can"
          && plural.systemd.services.dejana-service.serviceConfig.NetworkNamespacePath == "/run/netns/vpn-dejana";
        message = "plural VPN instances must create isolated interfaces and bind their own services";
      }
      {
        name = "plural-socks-loopback-only";
        assertion = lib.hasInfix "bind=127.0.0.1" plural.systemd.services."netns-vpn-can-forward-socks".serviceConfig.ExecStart;
        message = "plural SOCKS forwards must bind to host loopback by default";
      }
      {
        name = "plural-dynamic-endpoint-refresh";
        assertion = let
          peer = builtins.head plural.networking.wireguard.interfaces.wg-can.peers;
        in
          peer.dynamicEndpointRefreshSeconds
          == 30
          && peer.dynamicEndpointRefreshRestartSeconds == 5;
        message = "dynamic endpoint refresh settings must flow into the wireguard peer";
      }
      {
        name = "plural-app-sudo-users";
        assertion = lib.any (rule:
          rule.users
          == ["can"]
          && lib.any (command: lib.hasInfix "vpn-can-qbittorrent" command.command) rule.commands)
        plural.security.sudo.extraRules;
        message = "plural app launchers must restrict sudo access to their declared users";
      }
      {
        name = "plural-renewal-timer";
        assertion =
          plural.systemd.timers."netns-vpn-can-renewal".wantedBy
          == ["timers.target"]
          && plural.systemd.services."netns-vpn-can-renewal".environment.VPN_NAMESPACE == "vpn-can"
          && plural.systemd.services."netns-vpn-can-renewal".environment.VPN_PORT_FORWARDING_PROFILE
          == builtins.toJSON {
            type = "nat-pmp";
            gateway = "10.2.0.1";
          };
        message = "NAT-PMP renewal must run in the selected namespace with its profile data";
      }
    ];
  }
