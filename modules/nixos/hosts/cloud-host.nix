{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.cloudHost;
  inherit (lib) mkDefault mkIf optionalAttrs;
  mib = 1024 * 1024;
  isUefi = cfg.boot.mode == "uefi";
in {
  imports = [./cloud-host-options.nix ./cloud-host-network.nix ./cloud-host-storage.nix];

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = builtins.elem pkgs.stdenv.hostPlatform.system ["x86_64-linux" "aarch64-linux"];
        message = "cloudHost: only x86_64-linux and aarch64-linux guests are supported.";
      }
      {
        assertion = isUefi || pkgs.stdenv.hostPlatform.system == "x86_64-linux";
        message = "cloudHost: BIOS boot requires x86_64-linux.";
      }
      {
        assertion = cfg.access.authorizedKeys != [];
        message = "cloudHost: at least one administrative SSH public key is required.";
      }
      {
        assertion =
          config.services.openssh.enable
          && config.services.openssh.settings.PubkeyAuthentication
          && !config.services.openssh.settings.PasswordAuthentication
          && !config.services.openssh.settings.KbdInteractiveAuthentication
          && config.services.openssh.settings.PermitRootLogin == "prohibit-password";
        message = "cloudHost: SSH administration must remain key-only.";
      }
      {
        assertion = cfg.resources.maxFreeMiB > cfg.resources.minFreeMiB;
        message = "cloudHost: maxFreeMiB must exceed minFreeMiB.";
      }
    ];

    system.stateVersion = cfg.stateVersion;
    # QEMU/KVM describes guest hardware, not a cloud provider. Other platforms
    # supply their own initrd drivers through ordinary NixOS modules.
    boot.initrd.availableKernelModules = lib.optionals (cfg.platform == "qemu") [
      "virtio_pci"
      "virtio_blk"
      "virtio_scsi"
      "virtio_net"
      "sd_mod"
      "sr_mod"
    ];
    services.qemuGuest.enable = mkDefault (cfg.platform == "qemu");
    boot.growPartition = false; # Disko uses the complete selected disk at install.
    disko.devices.disk.system = {
      type = "disk";
      device = cfg.disk;
      content = {
        type = "gpt";
        partitions =
          (optionalAttrs isUefi {
            ESP = {
              size = "512M";
              type = "EF00";
              content = {
                type = "filesystem";
                format = "vfat";
                mountpoint = "/boot";
                mountOptions = ["umask=0077"];
              };
            };
          })
          // (optionalAttrs (!isUefi) {
            bios = {
              size = "1M";
              type = "EF02";
              priority = 1;
            };
          })
          // {
            root = {
              size = "100%";
              # The storage module owns the Btrfs content and rollback boundary.
            };
          };
      };
    };
    boot.loader = {
      systemd-boot = mkIf isUefi {
        enable = true;
        configurationLimit = cfg.resources.bootGenerations;
      };
      # Removable-path installation also boots guests without writable EFI
      # variables. Do not depend on a provider's NVRAM surviving replacement.
      efi.canTouchEfiVariables = false;
      grub = mkIf (!isUefi) {
        enable = true;
        configurationLimit = cfg.resources.bootGenerations;
        # Disko derives grub.devices from the selected disk's EF02 partition.
      };
    };

    services.openssh = {
      enable = true;
      ports = [cfg.access.port];
      openFirewall = true;
      settings = {
        PubkeyAuthentication = true;
        PasswordAuthentication = false;
        KbdInteractiveAuthentication = false;
        PermitRootLogin = "prohibit-password";
      };
    };
    users.mutableUsers = mkDefault false;
    users.users.root = {
      openssh.authorizedKeys.keys = cfg.access.authorizedKeys;
      hashedPassword = mkDefault "!";
    };
    networking.firewall.enable = true;
    networking.nftables.enable = true;

    nix.settings = {
      # The operator supplies a build host; the guest only installs closures.
      max-jobs = cfg.resources.localBuildJobs;
      min-free = cfg.resources.minFreeMiB * mib;
      max-free = cfg.resources.maxFreeMiB * mib;
    };
    nix.gc = {
      automatic = mkDefault true;
      dates = mkDefault "weekly";
      options = mkDefault "--delete-older-than 30d";
    };
    services.journald.settings.Journal = {
      SystemMaxUse = "${toString cfg.resources.journalMaxMiB}M";
      SystemKeepFree = "${toString cfg.resources.journalKeepFreeMiB}M";
      MaxRetentionSec = "7day";
    };
  };
}
