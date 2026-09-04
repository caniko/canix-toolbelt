{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mkEnableOption mkIf mkMerge mkOption types;

  legacyCfg = config.canix-toolbelt.networking.vpnNetns;
  pluralCfg = config.canix-toolbelt.networking.vpnNamespaces;
  identifierPattern = "^[A-Za-z0-9][A-Za-z0-9_-]*$";

  legacyPeerType = types.submodule {
    options = {
      publicKey = mkOption {
        type = types.singleLineStr;
        description = "Base64 public key of the VPN server peer.";
      };
      endpoint = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Server endpoint host:port.";
      };
      allowedIPs = mkOption {
        type = types.listOf types.str;
        default = ["0.0.0.0/0"];
      };
      persistentKeepalive = mkOption {
        type = types.nullOr types.int;
        default = 25;
      };
      dynamicEndpointRefreshSeconds = mkOption {
        type = types.nullOr types.int;
        default = 30;
      };
      dynamicEndpointRefreshRestartSeconds = mkOption {
        type = types.nullOr types.int;
        default = 5;
      };
    };
  };

  profilePeerType = types.submodule {
    options = {
      publicKey = mkOption {type = types.singleLineStr;};
      endpoint = mkOption {type = types.str;};
      allowedIps = mkOption {type = types.listOf types.str;};
      persistentKeepaliveSeconds = mkOption {
        type = types.nullOr types.ints.unsigned;
        default = null;
      };
      dynamicEndpointRefreshSeconds = mkOption {
        type = types.nullOr types.ints.unsigned;
        default = null;
      };
      dynamicEndpointRefreshRestartSeconds = mkOption {
        type = types.nullOr types.ints.unsigned;
        default = null;
      };
    };
  };

  profileType = types.submodule {
    options = {
      provider = mkOption {
        type = types.str;
        description = "Provider identifier carried through from the profile; not interpreted by this module.";
      };
      owner = mkOption {
        type = types.nullOr types.str;
        default = null;
      };
      dnsServers = mkOption {
        type = types.listOf types.str;
        default = [];
      };
      connection = mkOption {
        type = types.submodule {
          options = {
            type = mkOption {type = types.enum ["wireguard"];};
            addresses = mkOption {
              type = types.listOf types.str;
              default = [];
            };
            privateKeyRef = mkOption {
              type = types.singleLineStr;
              description = "External secret reference carried by Fleetix; privateKeyFile supplies the resolved local path.";
            };
            peers = mkOption {
              type = types.listOf profilePeerType;
              default = [];
            };
          };
        };
      };
      portForwarding = mkOption {
        type = types.nullOr (types.submodule {
          options = {
            type = mkOption {type = types.enum ["nat-pmp"];};
            gateway = mkOption {type = types.singleLineStr;};
          };
        });
        default = null;
        description = "Provider-neutral port-forwarding profile data made available to the optional renewal command.";
      };
    };
  };

  appType = allowedUsers:
    types.submodule {
      options =
        {
          package = mkOption {
            type = types.nullOr types.package;
            default = null;
            description = "Package whose binary should run inside the namespace.";
          };
          bin = mkOption {
            type = types.nullOr types.str;
            default = null;
            description = "Explicit executable path instead of package.";
          };
          icon = mkOption {
            type = types.nullOr types.str;
            default = null;
          };
          desktopName = mkOption {
            type = types.nullOr types.str;
            default = null;
          };
        }
        // lib.optionalAttrs allowedUsers {
          allowedUsers = mkOption {
            type = types.listOf types.str;
            default = [];
            description = "Users allowed to run this fixed launcher through sudo.";
          };
        };
    };

  pluralInstanceType = types.submodule {
    options = {
      profile = mkOption {
        type = profileType;
        description = "Fleetix-style VPN profile.";
      };
      privateKeyFile = mkOption {
        type = types.str;
        description = "Resolved local path to the WireGuard private key.";
      };
      boundServices = mkOption {
        type = types.listOf types.str;
        default = [];
      };
      portForwards = mkOption {
        type = types.attrsOf types.port;
        default = {};
        description = "Host TCP forwards into namespace-local loopback ports (label -> port).";
      };
      socks = {
        enable = mkEnableOption "a SOCKS5 proxy inside this VPN namespace";
        hostAddress = mkOption {
          type = types.str;
          default = "127.0.0.1";
          description = "Host address for the forwarded SOCKS5 listener.";
        };
        port = mkOption {
          type = types.port;
          default = 1080;
        };
      };
      wrappedApps = mkOption {
        type = types.attrsOf (appType true);
        default = {};
      };
      renewal = {
        command = mkOption {
          type = types.nullOr types.str;
          default = null;
          description = "Consumer-supplied one-shot port-forward renewal command.";
        };
        onBootSec = mkOption {
          type = types.str;
          default = "15s";
        };
        onUnitActiveSec = mkOption {
          type = types.str;
          default = "45s";
        };
        after = mkOption {
          type = types.listOf types.str;
          default = [];
          description = "Additional systemd units that must start before renewal.";
        };
      };
    };
  };

  appBin = app:
    if app.bin != null
    then app.bin
    else lib.getExe app.package;

  appDesktopName = label: app:
    if app.desktopName != null
    then app.desktopName
    else label;

  mkPluralInstance = id: cfg: let
    connection = cfg.profile.connection;
  in {
    inherit id cfg;
    legacy = false;
    namespace = "vpn-${id}";
    interface = "wg-${id}";
    addresses = connection.addresses;
    peers =
      map (peer: {
        inherit (peer) endpoint publicKey dynamicEndpointRefreshSeconds dynamicEndpointRefreshRestartSeconds;
        allowedIPs = peer.allowedIps;
        persistentKeepalive = peer.persistentKeepaliveSeconds;
      })
      connection.peers;
    allowedIps = lib.concatMap (peer: peer.allowedIps) connection.peers;
    inherit (cfg.profile) dnsServers portForwarding;
    inherit (cfg) boundServices portForwards wrappedApps renewal privateKeyFile;
    socks = cfg.socks.enable;
    socksHostAddress = cfg.socks.hostAddress;
    socksPort = cfg.socks.port;
  };

  legacyInstance = {
    id = "legacy";
    cfg = legacyCfg;
    legacy = true;
    namespace = legacyCfg.name;
    interface = legacyCfg.interfaceName;
    addresses = legacyCfg.ips;
    peers = legacyCfg.peers;
    allowedIps = lib.concatMap (peer: peer.allowedIPs) legacyCfg.peers;
    dnsServers = legacyCfg.dnsServers;
    portForwarding = null;
    inherit (legacyCfg) boundServices portForwards wrappedApps privateKeyFile;
    renewal = {
      command = null;
      after = [];
      onBootSec = "";
      onUnitActiveSec = "";
    };
    socks = legacyCfg.socks;
    socksHostAddress = null;
    socksPort = legacyCfg.socksPort;
  };

  instances =
    lib.mapAttrsToList mkPluralInstance pluralCfg
    ++ lib.optional legacyCfg.enable legacyInstance;

  namesFor = instance: let
    suffix =
      if instance.legacy
      then "vpn"
      else "vpn-${instance.id}";
  in {
    create = "netns-${suffix}";
    setup = "netns-${suffix}-setup";
    sockets = "netns-${suffix}-sockets";
    forward = label: "netns-${suffix}-forward-${label}";
    renewal = "netns-${suffix}-renewal";
  };

  forwardedPorts = instance:
    instance.portForwards
    // lib.optionalAttrs instance.socks {
      ${
        if instance.legacy
        then "microsocks"
        else "socks"
      } =
        instance.socksPort;
    };

  generatedServiceNames = instance: let
    names = namesFor instance;
  in
    [names.create names.setup]
    ++ lib.optional instance.socks names.sockets
    ++ map names.forward (lib.attrNames (forwardedPorts instance))
    ++ lib.optional (instance.renewal.command != null) names.renewal;

  allNamespaceNames = map (instance: instance.namespace) instances;
  allInterfaceNames = map (instance: instance.interface) instances;
  allBoundServices = lib.concatMap (instance: instance.boundServices) instances;
  allGeneratedServices = lib.concatMap generatedServiceNames instances;
  allHostPorts = lib.concatMap (instance: lib.attrValues (forwardedPorts instance)) instances;

  renderInstance = instance: let
    names = namesFor instance;
    netnsPath = "/run/netns/${instance.namespace}";
    resolvConf = "/etc/netns/${instance.namespace}/resolv.conf";
    hasIpv4 = lib.any (value: !(lib.hasInfix ":" value)) (instance.addresses ++ instance.allowedIps);
    hasIpv6 = lib.any (lib.hasInfix ":") (instance.addresses ++ instance.allowedIps);
    disableIpv6 =
      if hasIpv6
      then "0"
      else "1";
    setupUnit = "${names.setup}.service";

    legacyExec = pkgs.writeShellScriptBin "vpn-exec" ''
      user="''${SUDO_USER:-}"
      if [ -z "$user" ]; then
        echo "vpn-exec: run via sudo (canix-toolbelt.networking.vpnNetns installs a NOPASSWD rule)" >&2
        exit 1
      fi
      exec ${pkgs.iproute2}/bin/ip netns exec ${instance.namespace} \
        ${pkgs.util-linux}/bin/setpriv --reuid="$user" --regid="$user" --init-groups "$@"
    '';

    appLaunchers = lib.mapAttrs (label: app:
      pkgs.writeShellScriptBin "vpn-${instance.id}-${label}" ''
        user="''${SUDO_USER:-}"
        if [ -z "$user" ]; then
          echo "vpn-${instance.id}-${label}: run via sudo" >&2
          exit 1
        fi
        exec ${pkgs.iproute2}/bin/ip netns exec ${instance.namespace} \
          ${pkgs.util-linux}/bin/setpriv --reuid="$user" --regid="$user" --init-groups ${lib.escapeShellArg (appBin app)}
      '')
    instance.wrappedApps;

    desktopItems = lib.mapAttrsToList (label: app:
      pkgs.makeDesktopItem {
        name =
          if instance.legacy
          then label
          else "vpn-${instance.id}-${label}";
        desktopName =
          if instance.legacy
          then "${appDesktopName label app} (VPN)"
          else "${appDesktopName label app} (${instance.id} VPN)";
        exec =
          if instance.legacy
          then
            "${lib.getExe pkgs.sudo} -n --preserve-env=DISPLAY,XAUTHORITY,WAYLAND_DISPLAY,DBUS_SESSION_BUS_ADDRESS,SSH_AUTH_SOCK "
            + "${legacyExec}/bin/vpn-exec ${appBin app}"
          else let
            launcher = appLaunchers.${label};
          in
            "${lib.getExe pkgs.sudo} -n --preserve-env=DISPLAY,XAUTHORITY,WAYLAND_DISPLAY,DBUS_SESSION_BUS_ADDRESS,SSH_AUTH_SOCK "
            + "${launcher}/bin/vpn-${instance.id}-${label}";
        inherit (app) icon;
      })
    instance.wrappedApps;

    boundServices = lib.genAttrs instance.boundServices (_: {
      bindsTo = [setupUnit];
      after = [setupUnit];
      serviceConfig = {
        NetworkNamespacePath = netnsPath;
        BindReadOnlyPaths = ["${resolvConf}:/etc/resolv.conf"];
      };
    });

    portForwardUnits = lib.mapAttrs' (label: port: let
      bindAddress =
        if !instance.legacy && instance.socks && label == "socks"
        then instance.socksHostAddress
        else null;
    in
      lib.nameValuePair (names.forward label) {
        description = "Forward TCP port ${toString port} from the ${instance.namespace} VPN namespace to the host";
        wantedBy = ["multi-user.target"];
        bindsTo = [setupUnit];
        after = [setupUnit];
        serviceConfig = {
          ExecStart = ''
            ${pkgs.socat}/bin/socat TCP-LISTEN:${toString port}${lib.optionalString (bindAddress != null) ",bind=${bindAddress}"},fork,reuseaddr EXEC:'${pkgs.util-linux}/bin/nsenter --net=${netnsPath} ${pkgs.socat}/bin/socat STDIO TCP:127.0.0.1:${toString port}'
          '';
          Restart = "always";
        };
      })
    (forwardedPorts instance);

    appAssertions =
      lib.mapAttrsToList (label: app: {
        assertion =
          (app.package != null || app.bin != null)
          && builtins.match identifierPattern label != null
          && (instance.legacy || app.allowedUsers != []);
        message =
          if instance.legacy
          then "canix-toolbelt.networking.vpnNetns.wrappedApps.${label} requires package or bin and a safe identifier."
          else "canix-toolbelt.networking.vpnNamespaces.${instance.id}.wrappedApps.${label} requires package or bin, a safe identifier, and explicit identifier-safe allowedUsers.";
      })
      instance.wrappedApps;
  in {
    assertions =
      [
        {
          assertion = builtins.match identifierPattern instance.namespace != null;
          message = "VPN namespace `${instance.namespace}` must use only letters, digits, underscores, and hyphens.";
        }
        {
          assertion = builtins.match identifierPattern instance.interface != null;
          message = "WireGuard interface `${instance.interface}` must use only letters, digits, underscores, and hyphens.";
        }
        {
          assertion = builtins.stringLength instance.interface <= 15;
          message = "WireGuard interface `${instance.interface}` exceeds Linux's 15-byte interface-name limit.";
        }
        {
          assertion = instance.privateKeyFile != "";
          message = "VPN namespace `${instance.namespace}` requires privateKeyFile.";
        }
        {
          assertion = instance.addresses != [];
          message = "VPN namespace `${instance.namespace}` requires at least one connection address.";
        }
        {
          assertion = instance.peers != [];
          message = "VPN namespace `${instance.namespace}` requires at least one WireGuard peer.";
        }
        {
          assertion = instance.renewal.command == null || instance.portForwarding != null;
          message = "VPN namespace `${instance.namespace}` requires profile.portForwarding when renewal.command is set.";
        }
        {
          assertion = instance.dnsServers != [];
          message = "VPN namespace `${instance.namespace}` requires at least one DNS server.";
        }
        {
          assertion =
            !(
              instance.socks
              && instance.portForwards
                ? ${
                if instance.legacy
                then "microsocks"
                else "socks"
              }
            );
          message = "VPN namespace `${instance.namespace}` reserves its SOCKS port-forward label when SOCKS is enabled.";
        }
      ]
      ++ map (label: {
        assertion = builtins.match identifierPattern label != null;
        message = "VPN namespace `${instance.namespace}` port-forward label `${label}` is not a safe identifier.";
      }) (lib.attrNames instance.portForwards)
      ++ appAssertions;

    environment.etc."netns/${instance.namespace}/resolv.conf".text =
      lib.concatMapStringsSep "\n" (server: "nameserver ${server}") instance.dnsServers;

    environment.systemPackages =
      lib.optional instance.legacy legacyExec
      ++ lib.attrValues appLaunchers
      ++ desktopItems;

    networking.wireguard.interfaces.${instance.interface} = {
      ips = instance.addresses;
      inherit (instance) peers privateKeyFile;
      interfaceNamespace = instance.namespace;
    };

    systemd.services =
      {
        ${names.create} = {
          description = "Create the ${instance.namespace} VPN network namespace";
          before = ["network.target"];
          requiredBy = ["wireguard-${instance.interface}.service"];
          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
            ExecStart = pkgs.writeShellScript "${names.create}-create" ''
              set -eu
              ${pkgs.iproute2}/bin/ip netns add ${instance.namespace}
              trap '${pkgs.iproute2}/bin/ip netns del ${instance.namespace}' ERR
              ${pkgs.iproute2}/bin/ip netns exec ${instance.namespace} ${pkgs.procps}/bin/sysctl -w net.ipv6.conf.all.disable_ipv6=${disableIpv6}
              ${pkgs.iproute2}/bin/ip netns exec ${instance.namespace} ${pkgs.procps}/bin/sysctl -w net.ipv6.conf.default.disable_ipv6=${disableIpv6}
            '';
            ExecStop = "${pkgs.iproute2}/bin/ip netns del ${instance.namespace}";
          };
        };

        ${names.setup} = {
          description = "Configure networking and DNS in the ${instance.namespace} VPN namespace";
          after = ["${names.create}.service" "wireguard-${instance.interface}.service"];
          requires = ["${names.create}.service" "wireguard-${instance.interface}.service"];
          wantedBy = ["multi-user.target"];
          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
            ExecStart = pkgs.writeShellScript "${names.setup}-configure" ''
              set -eu
              ${pkgs.iproute2}/bin/ip -n ${instance.namespace} link set lo up
              ${pkgs.iproute2}/bin/ip netns exec ${instance.namespace} ${pkgs.procps}/bin/sysctl -w net.ipv6.conf.${instance.interface}.disable_ipv6=${disableIpv6}
              ${lib.optionalString hasIpv4 "${pkgs.iproute2}/bin/ip -n ${instance.namespace} route replace default dev ${instance.interface}"}
              ${lib.optionalString hasIpv6 "${pkgs.iproute2}/bin/ip -n ${instance.namespace} -6 route replace default dev ${instance.interface}"}
            '';
          };
        };

        ${names.sockets} = mkIf instance.socks {
          description = "SOCKS5 proxy in the ${instance.namespace} VPN namespace";
          wantedBy = ["multi-user.target"];
          bindsTo = [setupUnit];
          after = [setupUnit];
          serviceConfig = {
            ExecStart = "${pkgs.microsocks}/bin/microsocks -i 127.0.0.1 -p ${toString instance.socksPort}";
            Restart = "always";
            DynamicUser = true;
            NetworkNamespacePath = netnsPath;
          };
        };
      }
      // boundServices
      // portForwardUnits
      // {
        ${names.renewal} = mkIf (instance.renewal.command != null) {
          description = "Renew port forwarding for the ${instance.namespace} VPN namespace";
          after = [setupUnit] ++ instance.renewal.after;
          requires = [setupUnit];
          serviceConfig = {
            Type = "oneshot";
            ExecStart = instance.renewal.command;
            NetworkNamespacePath = netnsPath;
            BindReadOnlyPaths = ["${resolvConf}:/etc/resolv.conf"];
          };
          environment = {
            VPN_NAMESPACE = instance.namespace;
            VPN_INTERFACE = instance.interface;
            VPN_PORT_FORWARDING_PROFILE = builtins.toJSON instance.portForwarding;
          };
        };
      };

    systemd.timers.${names.renewal} = mkIf (instance.renewal.command != null) {
      wantedBy = ["timers.target"];
      timerConfig = {
        OnBootSec = instance.renewal.onBootSec;
        OnUnitActiveSec = instance.renewal.onUnitActiveSec;
        Unit = "${names.renewal}.service";
      };
    };
    security.sudo.extraRules =
      if instance.legacy
      then
        lib.optional (instance.wrappedApps != {}) {
          groups = legacyCfg.sudoGroups;
          commands = [
            {
              command = "${legacyExec}/bin/vpn-exec";
              options = ["NOPASSWD"];
            }
          ];
        }
      else
        lib.mapAttrsToList (label: app: let
          launcher = appLaunchers.${label};
        in {
          users = app.allowedUsers;
          commands = [
            {
              command = "${launcher}/bin/vpn-${instance.id}-${label}";
              options = ["NOPASSWD"];
            }
          ];
        })
        instance.wrappedApps;
  };

  renderedInstances = map renderInstance instances;
in {
  options.canix-toolbelt.networking = {
    vpnNamespaces = mkOption {
      type = types.attrsOf pluralInstanceType;
      default = {};
      description = "Provider-neutral VPN network namespace instances.";
    };

    vpnNetns = {
      enable = mkEnableOption "the legacy singular VPN WireGuard network namespace";
      name = mkOption {
        type = types.str;
        default = "vpn";
      };
      interfaceName = mkOption {
        type = types.str;
        default = "wg0";
      };
      ips = mkOption {
        type = types.listOf types.str;
        default = [];
      };
      secretName = mkOption {
        type = types.str;
        default = "wg-pk-surfshark";
      };
      privateKeyFile = mkOption {
        type = types.str;
        default = config.age.secrets.${legacyCfg.secretName}.path or "";
        defaultText = lib.literalExpression "config.age.secrets.${config.canix-toolbelt.networking.vpnNetns.secretName}.path";
      };
      peers = mkOption {
        type = types.listOf legacyPeerType;
        default = [];
      };
      dnsServers = mkOption {
        type = types.listOf types.str;
        default = [];
      };
      boundServices = mkOption {
        type = types.listOf types.str;
        default = [];
      };
      portForwards = mkOption {
        type = types.attrsOf types.port;
        default = {};
      };
      socks = mkEnableOption "a SOCKS5 proxy inside the legacy VPN namespace";
      socksPort = mkOption {
        type = types.port;
        default = 1080;
      };
      wrappedApps = mkOption {
        type = types.attrsOf (appType false);
        default = {};
      };
      sudoGroups = mkOption {
        type = types.listOf types.str;
        default = ["wheel"];
      };
    };
  };

  config = {
    assertions =
      [
        {
          assertion = builtins.length allNamespaceNames == builtins.length (lib.unique allNamespaceNames);
          message = "VPN namespace names must be unique across vpnNamespaces and vpnNetns.";
        }
        {
          assertion = builtins.length allInterfaceNames == builtins.length (lib.unique allInterfaceNames);
          message = "WireGuard interface names must be unique across vpnNamespaces and vpnNetns.";
        }
        {
          assertion = builtins.length allBoundServices == builtins.length (lib.unique allBoundServices);
          message = "A systemd service may be bound to only one VPN namespace.";
        }
        {
          assertion = lib.intersectLists allBoundServices allGeneratedServices == [];
          message = "VPN-bound service names must not collide with generated VPN namespace services.";
        }
        {
          assertion = builtins.length allHostPorts == builtins.length (lib.unique allHostPorts);
          message = "Host ports forwarded from VPN namespaces must be unique.";
        }
      ]
      ++ lib.concatMap (rendered: rendered.assertions) renderedInstances;

    environment.etc = mkMerge (map (rendered: rendered.environment.etc) renderedInstances);
    environment.systemPackages = lib.concatMap (rendered: rendered.environment.systemPackages) renderedInstances;
    networking.wireguard.interfaces = mkMerge (map (rendered: rendered.networking.wireguard.interfaces) renderedInstances);
    systemd.services = mkMerge (map (rendered: rendered.systemd.services) renderedInstances);
    systemd.timers = mkMerge (map (rendered: rendered.systemd.timers) renderedInstances);
    security.sudo.extraRules = lib.concatMap (rendered: rendered.security.sudo.extraRules) renderedInstances;
  };
}
