# VPN-enforced network for any set of apps or services.
#
# Creates a network namespace, brings a WireGuard tunnel up inside it
# (e.g. a commercial VPN such as Surfshark), and routes any number of systemd
# services or desktop applications through that one tunnel. Traffic from
# wrapped apps can only egress via the tunnel, so a dropped tunnel fails them
# closed.
#
# This is a generalisation of the canix *arr stack that used to live
# host-locally (root/hosts/atlas/server/media/arr/default.nix).
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.networking.vpnNetns;
  wgIface = cfg.interfaceName;
  netnsPath = "/run/netns/${cfg.name}";
  resolvConf = "/etc/netns/${cfg.name}/resolv.conf";

  # Pull an existing systemd service into the namespace.
  vpnService = _: {
    bindsTo = ["netns-vpn-setup.service"];
    after = ["netns-vpn-setup.service"];
    serviceConfig = {
      NetworkNamespacePath = netnsPath;
      BindReadOnlyPaths = ["${resolvConf}:/etc/resolv.conf"];
    };
  };

  # socat host:port -> netns 127.0.0.1:port so services inside the namespace
  # stay reachable on the host (mirrors the atlas mkArrPortForward helper).
  portForwardService = port: {
    description = "Forward a port from the ${cfg.name} VPN namespace to the host";
    wantedBy = ["multi-user.target"];
    bindsTo = ["netns-vpn-setup.service"];
    after = ["netns-vpn-setup.service"];
    serviceConfig = {
      ExecStart = ''
        ${pkgs.socat}/bin/socat TCP-LISTEN:${toString port},fork,reuseaddr EXEC:'${pkgs.util-linux}/bin/nsenter --net=${netnsPath} ${pkgs.socat}/bin/socat STDIO TCP:127.0.0.1:${toString port}'
      '';
      Restart = "always";
    };
  };

  # Launch an arbitrary executable inside the namespace. `ip netns exec` needs
  # root, so the launcher runs via sudo (NOPASSWD rule below); setpriv drops
  # back to the requesting user so GUI apps keep the caller's account.
  vpnExec = pkgs.writeShellScriptBin "vpn-exec" ''
    user="''${SUDO_USER:-}"
    if [ -z "$user" ]; then
      echo "vpn-exec: run via sudo (canix-toolbelt.networking.vpnNetns installs a NOPASSWD rule)" >&2
      exit 1
    fi
    exec ${pkgs.iproute2}/bin/ip netns exec ${cfg.name} \
      ${pkgs.util-linux}/bin/setpriv --reuid="$user" --regid="$user" --init-groups "$@"
  '';

  appBin = app:
    if app.bin != null
    then app.bin
    else lib.getExe app.package;

  appDesktopName = label: app:
    if app.desktopName != null
    then app.desktopName
    else label;

  forwardedPorts =
    cfg.portForwards
    // lib.optionalAttrs cfg.socks {microsocks = cfg.socksPort;};
in {
  options.canix-toolbelt.networking.vpnNetns = {
    enable = lib.mkEnableOption "a VPN WireGuard network namespace";

    name = lib.mkOption {
      type = lib.types.str;
      default = "vpn";
      description = "Name of the network namespace.";
    };

    interfaceName = lib.mkOption {
      type = lib.types.str;
      default = "wg0";
      description = "WireGuard interface placed inside the namespace.";
    };

    ips = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      example = ["10.14.0.2/16"];
      description = "Addresses (CIDR) assigned to the WireGuard interface by the VPN provider.";
    };

    secretName = lib.mkOption {
      type = lib.types.str;
      default = "wg-pk-surfshark";
      description = "Name of the agenix secret holding the WireGuard private key.";
    };

    privateKeyFile = lib.mkOption {
      type = lib.types.str;
      default = config.age.secrets.${cfg.secretName}.path or "";
      defaultText =
        lib.literalExpression
        "config.age.secrets.${config.canix-toolbelt.networking.vpnNetns.secretName}.path";
      description = "Path to the WireGuard private key file.";
    };

    peers = lib.mkOption {
      type = lib.types.listOf (lib.types.submodule {
        options = {
          publicKey = lib.mkOption {
            type = lib.types.singleLineStr;
            example = "uRT3uSAUwvm7Rp+s3n5V9JibsKHKAZ/8+SU3psG8QxI=";
            description = "Base64 public key of the VPN server peer.";
          };
          endpoint = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "ro-buc.prod.surfshark.com:51820";
            description = "Server endpoint host:port.";
          };
          allowedIPs = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = ["0.0.0.0/0"];
          };
          persistentKeepalive = lib.mkOption {
            type = lib.types.nullOr lib.types.int;
            default = 25;
          };
          dynamicEndpointRefreshSeconds = lib.mkOption {
            type = lib.types.nullOr lib.types.int;
            default = 30;
          };
          dynamicEndpointRefreshRestartSeconds = lib.mkOption {
            type = lib.types.nullOr lib.types.int;
            default = 5;
          };
        };
      });
      default = [];
      description = "WireGuard peers (the VPN server).";
    };

    dnsServers = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      example = ["162.252.172.57" "149.154.159.92"];
      description = "Nameservers written into the namespace's resolv.conf.";
    };

    boundServices = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      example = ["qbittorrent" "radarr"];
      description = "systemd service names moved into the VPN namespace.";
    };

    portForwards = lib.mkOption {
      type = lib.types.attrsOf lib.types.int;
      default = {};
      example = {qbittorrent = 8085;};
      description = "host:port -> netns 127.0.0.1:port socat forwards (label -> port).";
    };

    socks = lib.mkEnableOption "a SOCKS5 proxy (microsocks) inside the VPN namespace";

    socksPort = lib.mkOption {
      type = lib.types.int;
      default = 1080;
      description = "Host port of the in-namespace SOCKS5 proxy.";
    };

    wrappedApps = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule {
        options = {
          package = lib.mkOption {
            type = lib.types.nullOr lib.types.package;
            default = null;
            description = "Package whose binary should run inside the namespace.";
          };
          bin = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            description = "Explicit executable path instead of `package`.";
          };
          icon = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            description = "Icon name for the generated desktop entry.";
          };
          desktopName = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            description = "Display name (defaults to the attribute name).";
          };
        };
      });
      default = {};
      description = "Desktop apps launched inside the namespace via a generated '<Name> (VPN)' entry.";
    };

    sudoGroups = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = ["wheel"];
      description = "Groups allowed to NOPASSWD-run the vpn-exec launcher.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions =
      [
        {
          assertion = cfg.privateKeyFile != "";
          message = "canix-toolbelt.networking.vpnNetns requires a WireGuard private key (set privateKeyFile or declare age.secrets.${cfg.secretName}).";
        }
        {
          assertion = cfg.ips != [];
          message = "canix-toolbelt.networking.vpnNetns requires at least one WireGuard address (from the VPN provider).";
        }
        {
          assertion = cfg.peers != [];
          message = "canix-toolbelt.networking.vpnNetns requires at least one VPN server peer.";
        }
        {
          assertion = cfg.dnsServers != [];
          message = "canix-toolbelt.networking.vpnNetns requires dnsServers so wrapped apps can resolve names in the namespace.";
        }
      ]
      ++ (lib.mapAttrsToList (label: app: {
          assertion = app.package != null || app.bin != null;
          message = "canix-toolbelt.networking.vpnNetns.wrappedApps.${label} requires `package` or `bin`.";
        })
        cfg.wrappedApps);

    environment.etc."netns/${cfg.name}/resolv.conf".text = lib.concatMapStringsSep "\n" (ns: "nameserver ${ns}") cfg.dnsServers;
    environment.systemPackages =
      [vpnExec]
      ++ (lib.mapAttrsToList (label: app:
        pkgs.makeDesktopItem {
          name = label;
          desktopName = "${appDesktopName label app} (VPN)";
          exec =
            "${lib.getExe pkgs.sudo} -n --preserve-env=DISPLAY,XAUTHORITY,WAYLAND_DISPLAY,DBUS_SESSION_BUS_ADDRESS,SSH_AUTH_SOCK "
            + "${vpnExec}/bin/vpn-exec ${appBin app}";
          inherit (app) icon;
        })
      cfg.wrappedApps);

    networking.wireguard.interfaces.${wgIface} = {
      inherit (cfg) ips peers privateKeyFile;
      interfaceNamespace = cfg.name;
    };

    systemd.services = let
      boundServices = lib.listToAttrs (map (label: lib.nameValuePair label (vpnService label)) cfg.boundServices);
      portForwardUnits = lib.listToAttrs (map (label:
        lib.nameValuePair "netns-vpn-forward-${label}" (portForwardService forwardedPorts.${label}))
      (lib.attrNames forwardedPorts));
    in
      {
        # The namespace must exist before the wireguard service moves the
        # interface into it.
        netns-vpn = {
          description = "Create the ${cfg.name} VPN network namespace";
          before = ["network.target"];
          requiredBy = ["wireguard-${wgIface}.service"];
          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
            ExecStart = "${pkgs.iproute2}/bin/ip netns add ${cfg.name}";
            ExecStop = "${pkgs.iproute2}/bin/ip netns del ${cfg.name}";
          };
        };

        # loopback, routing and IPv6 inside the namespace. NixOS's wg peer unit
        # already installs the default route (allowed-ips); the explicit `route
        # replace` only keeps this unit idempotent.
        netns-vpn-setup = {
          description = "Configure networking and DNS in the ${cfg.name} VPN namespace";
          after = ["netns-vpn.service" "wireguard-${wgIface}.service"];
          requires = ["netns-vpn.service" "wireguard-${wgIface}.service"];
          wantedBy = ["multi-user.target"];
          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
            ExecStart = pkgs.writeShellScript "vpn-netns-setup" ''
              set -eu
              ${pkgs.iproute2}/bin/ip -n ${cfg.name} link set lo up
              ${pkgs.iproute2}/bin/ip -n ${cfg.name} route replace default dev ${wgIface}
              ${pkgs.iproute2}/bin/ip netns exec ${cfg.name} ${pkgs.procps}/bin/sysctl -w net.ipv6.conf.all.disable_ipv6=1
            '';
          };
        };

        # SOCKS5 proxy inside the namespace (a poor-man's way to route apps that
        # only support a proxy: point them at the forwarded host port).
        netns-vpn-sockets = lib.mkIf cfg.socks {
          description = "SOCKS5 proxy in the ${cfg.name} VPN namespace";
          wantedBy = ["multi-user.target"];
          bindsTo = ["netns-vpn-setup.service"];
          after = ["netns-vpn-setup.service"];
          serviceConfig = {
            ExecStart = "${pkgs.microsocks}/bin/microsocks -i 127.0.0.1 -p ${toString cfg.socksPort}";
            Restart = "always";
            DynamicUser = true;
            NetworkNamespacePath = netnsPath;
          };
        };
      }
      // boundServices
      // portForwardUnits;

    security.sudo.extraRules = lib.mkIf (cfg.wrappedApps != {}) [
      {
        groups = cfg.sudoGroups;
        commands = [
          {
            command = "${vpnExec}/bin/vpn-exec";
            options = ["NOPASSWD"];
          }
        ];
      }
    ];
  };
}
