{config, lib, ...}: let
  inherit (lib) mkOption types;
  deviceTypes = import ../../../lib/deviceTypes.nix;

  hostUserSubmodule = types.submodule {
    freeformType = types.attrsOf types.anything;
    options = {
      hasAccount = mkOption {
        type = types.bool;
        default = false;
        description = "Whether this user has a login/home-manager profile on this host.";
      };

      personalPc = mkOption {
        type = types.bool;
        default = false;
        description = "Whether this host is a personal/primary workstation for this user.";
      };
    };
  };

  hostSubmodule = types.submodule {
    options = {
      hostPubkey = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA...";
        description = "SSH host public key (ed25519) for this machine.";
      };

      hostNames = mkOption {
        type = types.listOf types.str;
        default = [];
        example = ["myhost" "myhost.local" "192.168.1.1"];
        description = "Additional hostnames/IPs for SSH known_hosts.";
      };

      lanIp = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "10.0.0.72";
        description = "LAN IP address of this host.";
      };

      lanBroadcast = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "10.0.0.255";
        description = "LAN broadcast address (for Wake-on-LAN).";
      };

      wgHomeIp = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "10.0.0.2";
        description = "WireGuard wg-home VPN IP address of this host.";
      };

      wgHomePublicKey = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "vL3WZq8PdnCRxYU8Glo3wRP8kBd7x63ynYsiFrsRURg=";
        description = "WireGuard public key. Set on the wg-home server; clients pin this.";
      };

      macAddress = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "AA:BB:CC:DD:EE:FF";
        description = "MAC address of the primary network interface (for Wake-on-LAN).";
      };

      directLinkIp = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "10.0.0.1";
        description = "Static IP for direct P2P ethernet link.";
      };

      directLinkMac = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "AA:BB:CC:DD:EE:FF";
        description = "MAC address of the direct-link adapter (for interface matching).";
      };

      directLinkInterface = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "enp13s0";
        description = "Interface name for the direct P2P ethernet link.";
      };

      directLinkPeers = mkOption {
        type = types.listOf types.str;
        default = [];
        example = ["nomad"];
        description = "Hostnames this host is directly cabled to (mutual).";
      };

      wakeOnLanInterface = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "enp4s0";
        description = "Primary interface name to configure for Wake-on-LAN.";
      };

      deviceType = mkOption {
        type = types.enum deviceTypes;
        example = "server";
        description = "Device class. Drives per-host modules and gating in nexus.toggleSubmodule.deviceTypes.";
      };

      dataRoot = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "/data/nvme0/can";
        description = "Absolute path to this host's user-data filesystem root; the Projects directory convention is dataRoot/Projects.";
      };

      gpuIgpu = mkOption {
        type = types.nullOr (types.enum ["amd" "intel"]);
        default = null;
        description = "Integrated GPU vendor.";
      };

      gpuDgpu = mkOption {
        type = types.nullOr (types.enum ["amd" "intel" "nvidia"]);
        default = null;
        description = "Discrete GPU vendor.";
      };

      users = mkOption {
        type = types.attrsOf hostUserSubmodule;
        default = {};
        description = "Per-host user membership metadata used by RBAC.";
      };
    };
  };
in {
  options.canix-toolbelt.networking.links = mkOption {
    type = types.attrsOf (types.submodule {
      options = {
        cidr = mkOption {
          type = types.nullOr types.str;
          default = null;
          description = "Network CIDR for this link (e.g. \"10.123.0.0/24\").";
        };
        serverAddress = mkOption {
          type = types.nullOr types.str;
          default = null;
          description = "Address of the server node on this link.";
        };
        port = mkOption {
          type = types.nullOr types.port;
          default = null;
          description = "UDP port for this link (WireGuard etc.).";
        };
      };
    });
    default = {};
    description = "Declared network links with their derived CIDRs and server addresses. Populated by fleetix topology.";
  };

  options.canix-toolbelt.fleetix = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable fleetix integration: populates canix-toolbelt.hosts and networking.links from config.fleetix.topology.hosts. Requires the fleetix flake to be imported.";
    };
  };

  options.canix-toolbelt.hosts = mkOption {
    type = types.attrsOf hostSubmodule;
    default = {};
    description = "Central registry of all hosts with their SSH public keys and network addresses.";
  };

  config = lib.mkIf config.canix-toolbelt.fleetix.enable (let
    ft = config.fleetix.topology;
  in {
    canix-toolbelt.hosts = lib.mapAttrs (name: host: let
      ln = host.links or {};
    in {
      hostPubkey = host.hostPubkey or null;
      hostNames = host.hostNames or [];
      deviceType = host.deviceType or null;
      dataRoot = (host.storage or {}).dataRoot or null;
      users = host.users or {};
      gpuIgpu = (host.gpu or {}).igpu or null;
      gpuDgpu = (host.gpu or {}).dgpu or null;
      lanIp = host.network.lanIp or null;
      lanBroadcast = host.network.lanBroadcast or null;
      wgHomeIp = (ln.wg-home or {}).address or null;
      wgHomePublicKey = (ln.wg-home or {}).publicKey or null;
      macAddress = host.network.macAddress or null;
      directLinkIp = (ln.direct-link or {}).address or null;
      directLinkMac = (ln.direct-link or {}).macAddress or null;
      directLinkInterface = (ln.direct-link or {}).externalInterface or null;
      directLinkPeers = host.network.directLinkPeers or [];
      wakeOnLanInterface = host.network.wakeOnLanInterface or null;
    }) ft.hosts;
  });
}
