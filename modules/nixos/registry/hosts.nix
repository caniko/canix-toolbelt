{lib, ...}: let
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
        example = "192.168.178.72";
        description = "LAN IP address of this host.";
      };

      lanBroadcast = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "192.168.178.255";
        description = "LAN broadcast address (for Wake-on-LAN).";
      };

      wgHomeIp = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "10.123.0.2";
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
        example = "10.10.0.1";
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
  options.canix-toolbelt.hosts = mkOption {
    type = types.attrsOf hostSubmodule;
    default = {};
    description = "Central registry of all hosts with their SSH public keys and network addresses.";
  };
}
