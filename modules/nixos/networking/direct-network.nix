{
  config,
  lib,
  options,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.networking.directNetwork;
  hasHomeManager = lib.hasAttrByPath ["home-manager" "users"] options;
  safeSlice = "[A-Za-z0-9_]+(-[A-Za-z0-9_]+)*\\.slice";
  sliceType = lib.types.strMatching safeSlice;
  directSlice = parent: "${lib.removeSuffix ".slice" parent}-direct.slice";
  serviceSlice = enrolled: parent:
    if builtins.elem parent enrolled
    then parent
    else directSlice parent;
  # systemd slice names encode every resource ancestor in their hyphen prefixes.
  slicePath = slice: let
    parts = lib.splitString "-" (lib.removeSuffix ".slice" slice);
  in
    lib.concatStringsSep "/" (lib.genList (i: "${lib.concatStringsSep "-" (lib.take (i + 1) parts)}.slice") (builtins.length parts));
  rootSlices = lib.unique (["system-direct.slice"] ++ cfg.systemSlices ++ map (serviceSlice cfg.systemSlices) (lib.attrValues cfg.systemServices));
  userSlices = user: lib.unique (user.slices ++ map (serviceSlice user.slices) (lib.attrValues user.services));
  anchor = slice: "direct-network-${lib.removeSuffix ".slice" slice}";
  binary = "${cfg.package}/bin/canix-toolbelt-direct-network";
  ready = "${binary} ready";
  policyData = {
    socket = "/run/direct-network/ready.sock";
    serversFile = "/run/direct-network/servers";
    dnsCgroup = "system.slice/system-direct.slice/direct-network-dns.service";
    ip = "${pkgs.iproute2}/bin/ip";
    nft = "${pkgs.nftables}/bin/nft";
    nmcli = "${pkgs.networkmanager}/bin/nmcli";
    systemctl = "${config.systemd.package}/bin/systemctl";
    cgroupPaths =
      map slicePath rootSlices
      ++ lib.concatLists (lib.mapAttrsToList (name: user:
        map (slice: "user.slice/user-${toString config.users.users.${name}.uid}.slice/user@${toString config.users.users.${name}.uid}.service/${slicePath slice}") (userSlices user))
      cfg.users);
    allowedUsers = map (name: config.users.users.${name}.uid) (lib.attrNames cfg.users);
    inherit (cfg) directInterfaces dnsServers dnsZones networkManagerDns;
  };
  policy = pkgs.writeText "direct-network-policy.json" (builtins.toJSON policyData);
  mkUser = user: {
    systemd.user.slices = lib.genAttrs (map (lib.removeSuffix ".slice") (userSlices user)) (_: {});
    systemd.user.services = lib.mkMerge [
      (lib.genAttrs (map anchor (userSlices user)) (name: let
        slice = "${lib.removePrefix "direct-network-" name}.slice";
      in {
        Unit.Description = "Keep ${slice} enrolled in direct network routing";
        Service = {
          Type = "oneshot";
          RemainAfterExit = true;
          ExecStart = ready;
          Slice = slice;
          Restart = "on-failure";
          RestartSec = "2s";
          TimeoutStartSec = "20s";
        };
        Install.WantedBy = ["default.target"];
      }))
      (lib.mapAttrs (_: parent: {
          Unit = {
            # Include worker anchors: systemd-run workers do not inherit their
            # launching service's cgroup, but do retain their dedicated AMC slice.
            Requires = map (slice: "${anchor slice}.service") (userSlices user);
            After = map (slice: "${anchor slice}.service") (userSlices user);
          };
          Service.Slice = lib.mkForce (serviceSlice user.slices parent);
        })
        user.services)
    ];
  };
in {
  options.canix-toolbelt.networking.directNetwork = {
    enable = lib.mkEnableOption "independent direct routing and DNS for enrolled background workloads";
    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ../../../nix/package.nix {directNetwork = true;};
      description = "Toolbelt package containing the direct-network daemon and readiness client.";
    };
    users = lib.mkOption {
      default = {};
      type = lib.types.attrsOf (lib.types.submodule {
        options = {
          services = lib.mkOption {
            type = lib.types.attrsOf sliceType;
            default = {};
            description = "Enabled Home Manager service names mapped to their existing parent resource slices.";
          };
          slices = lib.mkOption {
            type = lib.types.listOf sliceType;
            default = [];
            description = "Dedicated background-worker slices; desktop application slices must not be enrolled.";
          };
        };
      });
    };
    systemServices = lib.mkOption {
      type = lib.types.attrsOf sliceType;
      default = {};
      description = "Enabled system-manager services mapped to their existing parent resource slices.";
    };
    systemSlices = lib.mkOption {
      type = lib.types.listOf sliceType;
      default = [];
      description = "Existing dedicated system background slices; retain native builder/resource paths.";
    };
    directInterfaces = lib.mkOption {
      type = lib.types.listOf (lib.types.strMatching "[A-Za-z0-9_.-]{1,15}");
      default = [];
      description = "Additional direct interfaces, such as fleet WireGuard. Physical uplinks are discovered live.";
    };
    dnsServers = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "Explicit literal-IP public resolvers in addition to current NetworkManager uplink DNS.";
    };
    dnsZones = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = {};
      description = "Fleet DNS domains mapped to literal-IP direct resolvers.";
    };
    networkManagerDns = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Use live physical-uplink DNS from NetworkManager.";
    };
  };

  config = lib.mkIf cfg.enable ({
      assertions = [
        {
          assertion = cfg.users == {} || hasHomeManager;
          message = "directNetwork.users requires integrated Home Manager.";
        }
        {
          assertion = lib.all (name: config.users.users.${name}.uid != null) (lib.attrNames cfg.users);
          message = "directNetwork requires explicit UIDs for enrolled user managers.";
        }
        {
          assertion =
            lib.all (slice: !(builtins.elem slice ["app.slice" "background.slice" "session.slice"]))
            (lib.concatMap (user: user.slices) (lib.attrValues cfg.users));
          message = "directNetwork must not exempt desktop/session application slices.";
        }
        {
          assertion = !(builtins.elem "system.slice" cfg.systemSlices) && !(builtins.elem "user.slice" cfg.systemSlices);
          message = "directNetwork requires scoped system background slices.";
        }
      ];
      networking.nftables.enable = true;
      # NSS resolve delegates over AF_UNIX and loses the originating cgroup.
      # Both policies instead query the host stub over DNS: only enrolled DNS
      # packets are redirected, while desktop DNS retains resolved's VPN policy.
      system.nssDatabases.hosts = lib.mkForce ["files" "mymachines" "myhostname" "dns"];
      environment.etc."direct-network-policy.json".text = builtins.toJSON policyData;
      environment.systemPackages = [cfg.package];
      systemd.slices = lib.genAttrs (map (lib.removeSuffix ".slice") rootSlices) (_: {});
      systemd.services = lib.mkMerge [
        {
          direct-network = {
            description = "Maintain direct routes and service DNS independently of host VPNs";
            wantedBy = ["multi-user.target"];
            after = ["NetworkManager.service" "nftables.service"];
            wants = ["NetworkManager.service"];
            serviceConfig = {
              ExecStart = "${binary} daemon --policy ${policy}";
              ExecStartPre = "${pkgs.coreutils}/bin/rm -f /run/direct-network/ready.sock";
              RuntimeDirectory = "direct-network";
              RuntimeDirectoryMode = "0755";
              Restart = "on-failure";
              RestartSec = "2s";
              ProtectSystem = "strict";
              ProtectHome = true;
              ReadWritePaths = ["/run/direct-network"];
              PrivateTmp = true;
              CapabilityBoundingSet = ["CAP_NET_ADMIN"];
              NoNewPrivileges = true;
            };
          };
          direct-network-dns = {
            description = "Independent DNS for direct-network background services";
            wantedBy = ["multi-user.target"];
            requires = ["direct-network.service"];
            after = ["direct-network.service"];
            serviceConfig = {
              ExecStartPre = "${ready} --routes-only";
              ExecStart = "${pkgs.dnsmasq}/bin/dnsmasq --keep-in-foreground --conf-file=/dev/null --no-resolv --no-hosts --bind-interfaces --listen-address=127.0.0.54,::1 --port=5354 --servers-file=/run/direct-network/servers --pid-file=";
              Slice = "system-direct.slice";
              Restart = "on-failure";
              RestartSec = "2s";
              ProtectSystem = "strict";
              ProtectHome = true;
              PrivateTmp = true;
              NoNewPrivileges = true;
            };
          };
        }
        (lib.genAttrs (map anchor rootSlices) (name: {
          description = "Keep the direct background resource slice enrolled";
          requires = ["direct-network-dns.service"];
          after = ["direct-network-dns.service"];
          wantedBy = ["multi-user.target"];
          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
            ExecStart = ready;
            Slice = "${lib.removePrefix "direct-network-" name}.slice";
            Restart = "on-failure";
            RestartSec = "2s";
          };
        }))
        (lib.mapAttrs (_: parent: {
            requires = ["${anchor (serviceSlice cfg.systemSlices parent)}.service"];
            after = ["${anchor (serviceSlice cfg.systemSlices parent)}.service"];
            serviceConfig.Slice = lib.mkForce (serviceSlice cfg.systemSlices parent);
          })
          cfg.systemServices)
      ];
    }
    // lib.optionalAttrs hasHomeManager {
      home-manager.users = lib.mapAttrs (_: mkUser) cfg.users;
    });
}
