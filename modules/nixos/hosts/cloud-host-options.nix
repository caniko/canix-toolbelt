{lib, ...}: let
  inherit (lib) mkOption types;
  inherit (import ../../../lib/network-types.nix {inherit lib;}) ipType cidrType;
  addressFamily = family:
    types.submodule {
      options = {
        method = mkOption {
          type = types.enum ["auto" "manual" "disabled"];
          default =
            if family == "ipv4"
            then "auto"
            else "disabled";
          description = "NetworkManager addressing method for ${family}.";
        };
        addresses = mkOption {
          type = types.listOf (cidrType family);
          default = [];
          description = "Literal ${family} addresses with prefix lengths for manual addressing.";
        };
        gateway = mkOption {
          type = types.nullOr (ipType family);
          default = null;
          description = "Literal ${family} default gateway; required for manual addressing.";
        };
        gatewayOnLink = mkOption {
          type = types.bool;
          default = false;
          description = "Treat the default gateway as directly reachable, including with a /32 or /128 address.";
        };
        dns = mkOption {
          type = types.listOf (ipType family);
          default = [];
          description = "Literal DNS server addresses for this family; required for manual addressing.";
        };
      };
    };
in {
  options.canix-toolbelt.cloudHost = {
    enable = lib.mkEnableOption "a small, key-managed cloud NixOS host (requires the Disko module)";
    stateVersion = mkOption {
      type = types.strMatching "[0-9][0-9]\\.(05|11)";
      description = "Initial NixOS state version, pinned for this host's lifetime.";
    };
    platform = mkOption {
      type = types.enum ["qemu" "custom"];
      default = "qemu";
      description = "Guest hardware profile. custom requires caller-supplied boot drivers.";
    };
    disk = mkOption {
      type = types.strMatching "/dev/[^[:space:]]+";
      example = "/dev/disk/by-id/virtio-root";
      description = "Explicit whole installation disk. Disko destroys its contents only when the operator runs installation.";
    };
    boot.mode = mkOption {
      type = types.enum ["uefi" "bios"];
      description = "Firmware actually provided by the guest platform; BIOS is x86_64 only.";
    };
    storage = {
      compressionLevel = mkOption {
        type = types.ints.between 1 15;
        default = 3;
        description = "Btrfs Zstd compression level, applied consistently to all subvolumes.";
      };
      snapshots = {
        enable = mkOption {
          type = types.bool;
          default = true;
          description = "Daily Snapper root snapshots, excluding store, service state, logs and SSH host identity.";
        };
        dailyLimit = mkOption {
          type = types.ints.positive;
          default = 7;
          description = "Maximum daily root snapshots retained by timeline cleanup.";
        };
        numberLimit = mkOption {
          type = types.ints.positive;
          default = 5;
          description = "Maximum numbered manual snapshots retained by number cleanup. Protect the matching Nix closure separately.";
        };
      };
    };
    network = {
      interfaceName = mkOption {
        type = types.nullOr (types.strMatching "[a-zA-Z0-9_.:-]+");
        default = null;
        description = "Verified uplink name. Set this, macAddress, or both; no provider NIC name is assumed.";
      };
      macAddress = mkOption {
        type = types.nullOr (types.strMatching "([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}");
        default = null;
        description = "Verified uplink MAC address, permitting name-independent matching.";
      };
      mtu = mkOption {
        type = types.ints.between 576 9000;
        default = 1500;
        description = "Outer uplink MTU. Tunnel roles select their own measured inner MTU.";
      };
      ipv4 = mkOption {
        type = addressFamily "ipv4";
        default = {};
        description = "IPv4 uplink configuration.";
      };
      ipv6 = mkOption {
        type = addressFamily "ipv6";
        default = {};
        description = "IPv6 uplink configuration; disabled until the complete public path is validated.";
      };
    };
    access = {
      port = mkOption {
        type = types.port;
        default = 22;
        description = "Guest administrative SSH port. The gateway's public Git SSH listener must use a different port.";
      };
      authorizedKeys = mkOption {
        type = types.listOf (types.strMatching "(ssh-|ecdsa-|sk-)[^\n]+\n?");
        apply = map (lib.removeSuffix "\n");
        default = [];
        description = "Operator public keys for root administration, including before tunnel enrollment.";
      };
    };
    resources = {
      localBuildJobs = mkOption {
        type = types.ints.unsigned;
        default = 0;
        description = "Maximum local Nix build jobs; zero installs externally built closures only.";
      };
      minFreeMiB = mkOption {
        type = types.ints.unsigned;
        default = 256;
        description = "Nix garbage-collection low-water mark in MiB.";
      };
      maxFreeMiB = mkOption {
        type = types.ints.positive;
        default = 1024;
        description = "Nix garbage-collection target free space in MiB.";
      };
      bootGenerations = mkOption {
        type = types.ints.between 2 100;
        default = 5;
        description = "Maximum boot menu generations. This does not protect old generations from age-based GC.";
      };
      journalMaxMiB = mkOption {
        type = types.ints.positive;
        default = 128;
        description = "Maximum persistent journal size in MiB.";
      };
      journalKeepFreeMiB = mkOption {
        type = types.ints.positive;
        default = 256;
        description = "Minimum free space journald should leave in MiB.";
      };
    };
  };
}
