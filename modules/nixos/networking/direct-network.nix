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
  # RootDirectory/RootImage apply even to '+' commands. Keep readiness inline so
  # denial follows the payload's Restart= policy, and supply only the client's
  # runtime closure plus the live host socket directory inside alternate roots.
  rootedConfig = service:
    (service.RootDirectory or "") != "" || (service.RootImage or "") != "";
  rootedService = name: rootedConfig config.systemd.services.${name}.serviceConfig;
  binary = "${cfg.package}/bin/canix-toolbelt-direct-network";
  ready = "${binary} ready";
  # Empty systemd assignments clear all preceding commands. Preserve the
  # authored resets, then admit before the hooks that are actually effective.
  insertAdmission = command: original: let
    commands = lib.toList original;
    reset = lib.lists.findFirstIndex (value: builtins.isString value && lib.trim value == "") null (lib.reverseList commands);
    boundary = builtins.length commands - reset;
  in
    if reset == null
    then [command] ++ commands
    else lib.take boundary commands ++ [command] ++ lib.drop boundary commands;
  rootedAdmission = pkgs.runCommand "direct-network-rooted-admission.conf" {} ''
    printf '[Service]\nBindReadOnlyPaths=/run/direct-network\n' > "$out"
    while IFS= read -r path; do
      printf 'BindReadOnlyPaths=%s\n' "$path" >> "$out"
    done < ${(pkgs.closureInfo {rootPaths = [cfg.package];})}/store-paths
  '';
  rootedServices = lib.filterAttrs (name: _: rootedService name) cfg.systemServices;
  # The NixOS systemd module owns /etc/systemd/system as one immutable tree.
  # Compose generated drop-ins through its package interface, never nested etc
  # entries that would write through that tree's store-directory symlink.
  rootedAdmissionUnits = pkgs.runCommand "direct-network-rooted-admission-units" {} (
    lib.concatMapStringsSep "\n" (name: ''
      directory="$out"/${lib.escapeShellArg "lib/systemd/system/${name}.service.d"}
      mkdir -p "$directory"
      ln -s ${rootedAdmission} "$directory/50-direct-network-rooted-admission.conf"
    '') (lib.attrNames rootedServices)
  );
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
  mkUserConfig = user: {
    # An empty Home Manager unit is a masked unit to systemd. Every declared
    # slice needs content even when it inherits all its resource policy.
    systemd.user.slices = lib.genAttrs (map (lib.removeSuffix ".slice") (userSlices user)) (name: {
      Unit.Description = lib.mkDefault "Direct-network resource slice ${name}";
    });
    systemd.user.services = lib.mkMerge [
      (lib.genAttrs (map anchor (userSlices user)) (name: let
        slice = "${lib.removePrefix "direct-network-" name}.slice";
      in {
        Unit.Description = "Keep ${slice} enrolled in direct network routing";
        Service = {
          Type = "exec";
          ExecStartPre = ready;
          ExecStart = "${binary} anchor";
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
          Service = {
            Slice = lib.mkForce (serviceSlice user.slices parent);
          };
        })
        user.services)
    ];
  };
  mkUser = user: {config, ...}: {
    # Home Manager leaves ExecStartPre in its freeform unit schema, whose
    # scalar/list union cannot merge a scalar command with our ordered list.
    # Coerce commands only for enrolled services. Keep the option tree static:
    # submodule name is supplied through _module.args after options are loaded.
    options.systemd.user.services = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule ({name, ...}: {
        options.Service.ExecStartPre = lib.mkOption {
          type = let
            command = lib.types.either lib.types.str lib.types.path;
            primitive = lib.types.oneOf [lib.types.bool lib.types.int lib.types.str lib.types.path];
          in
            if builtins.hasAttr name user.services
            then lib.types.coercedTo command lib.toList (lib.types.listOf command)
            else lib.types.either primitive (lib.types.listOf primitive);
          default = [];
          apply = commands:
            if builtins.hasAttr name user.services
            then insertAdmission ready commands
            else commands;
          description = "Pre-start commands, with direct-network admission for enrolled services.";
        };
      }));
    };
    config =
      mkUserConfig user
      // {
        xdg.configFile = lib.mapAttrs' (name: _:
          lib.nameValuePair "systemd/user/${name}.service.d/50-direct-network-rooted-admission.conf" (lib.mkIf (rootedConfig config.systemd.user.services.${name}.Service) {
            source = rootedAdmission;
          }))
        user.services;
      };
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

  # glibc nscd has no per-database environment switch. Transform the final
  # operator-provided config so other database policies remain intact.
  options.services.nscd.config = lib.mkOption {
    apply = content:
      if cfg.enable
      then content + "\nenable-cache hosts no\nshared hosts no\n"
      else content;
  };

  # Inspect the fully merged freeform config only after its definitions are
  # resolved. Conditional ExecStartPre definitions that read their own sibling
  # RootDirectory/RootImage fields recurse while merging serviceConfig.
  options.systemd.services = lib.mkOption {
    type = lib.types.attrsOf (lib.types.submodule ({name, ...}: {
      options.serviceConfig = lib.mkOption {
        apply = service:
          if cfg.enable && builtins.hasAttr name cfg.systemServices
          then service // {ExecStartPre = insertAdmission "+${ready}" (service.ExecStartPre or []);}
          else service;
      };
    }));
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
      # Kernel reverse-path validation must use the direct mark restored on
      # replies and classified listener traffic, even with a full-tunnel default.
      boot.kernel.sysctl."net.ipv4.conf.all.src_valid_mark" = 1;
      # NSS resolve delegates over AF_UNIX and loses the originating cgroup.
      # Both policies instead query the host stub over DNS: only enrolled DNS
      # packets are redirected, while desktop DNS retains resolved's VPN policy.
      system.nssDatabases.hosts = lib.mkForce ["files" "mymachines" "myhostname" "dns"];
      environment.etc."direct-network-policy.json".text = builtins.toJSON policyData;
      systemd.packages = lib.optional (rootedServices != {}) rootedAdmissionUnits;
      environment.systemPackages = [cfg.package];
      users.groups.direct-network-dns = {};
      users.users.direct-network-dns = {
        isSystemUser = true;
        group = "direct-network-dns";
      };
      systemd.slices = lib.genAttrs (map (lib.removeSuffix ".slice") rootSlices) (name: {
        description = lib.mkDefault "Direct-network resource slice ${name}";
      });
      systemd.services = lib.mkMerge [
        {
          # nsncd 1.5.2's documented per-database switch keeps account lookups
          # proxied while glibc's files/DNS hostname lookups retain the caller.
          nscd = lib.mkIf config.services.nscd.enable {
            environment.NSNCD_IGNORE_HOSTS = "true";
          };
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
              # Authenticate the unprivileged resolver's MainPID FD and network
              # namespace through cross-UID, ptrace-controlled proc reads.
              CapabilityBoundingSet = ["CAP_NET_ADMIN" "CAP_SYS_PTRACE"];
              NoNewPrivileges = true;
            };
          };
          direct-network-dns = {
            description = "Independent DNS for direct-network background services";
            wantedBy = ["multi-user.target"];
            requires = ["direct-network.service"];
            after = ["direct-network.service"];
            serviceConfig = {
              User = "direct-network-dns";
              Group = "direct-network-dns";
              # Routes bootstrap is root-only; the network-facing payload starts
              # with its dedicated identity already established by systemd.
              ExecStartPre = "+${ready} --routes-only";
              ExecStart = "${pkgs.dnsmasq}/bin/dnsmasq --keep-in-foreground --user=direct-network-dns --group=direct-network-dns --conf-file=/dev/null --no-resolv --no-hosts --bind-interfaces --listen-address=127.0.0.54,::1 --port=5354 --servers-file=/run/direct-network/servers --pid-file=";
              Slice = "system-direct.slice";
              Restart = "on-failure";
              RestartSec = "2s";
              ProtectSystem = "strict";
              ProtectHome = true;
              PrivateTmp = true;
              NoNewPrivileges = true;
              # DNS-only unprivileged port needs no capabilities. Keep the
              # root-owned servers directory/file readable but not writable.
              # An empty list emits no directive, retaining systemd's default
              # capabilities instead of the explicit empty capability reset.
              CapabilityBoundingSet = "";
            };
          };
        }
        (lib.genAttrs (map anchor rootSlices) (name: {
          description = "Keep the direct background resource slice enrolled";
          wants = ["direct-network-dns.service"];
          after = ["direct-network-dns.service"];
          wantedBy = ["multi-user.target"];
          serviceConfig = {
            Type = "exec";
            ExecStartPre = ready;
            ExecStart = "${binary} anchor";
            Slice = "${lib.removePrefix "direct-network-" name}.slice";
            Restart = "on-failure";
            RestartSec = "2s";
          };
        }))
        (lib.mapAttrs (_: parent: {
            requires = ["${anchor (serviceSlice cfg.systemSlices parent)}.service"];
            after = ["${anchor (serviceSlice cfg.systemSlices parent)}.service"];
            serviceConfig = {
              Slice = lib.mkForce (serviceSlice cfg.systemSlices parent);
              # Active anchors retain established workloads across DNS failure.
              # Every new start still needs its own admission barrier. The narrow
              # readiness client runs as root even when User is not enrolled; it
              # cannot mutate the daemon's policy.
            };
          })
          cfg.systemServices)
      ];
    }
    // lib.optionalAttrs hasHomeManager {
      home-manager.users = lib.mapAttrs (_: mkUser) cfg.users;
    });
}
