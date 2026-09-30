{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.cloudHost;
  mounts = {
    "@root" = "/";
    "@nix" = "/nix";
    "@state" = "/var/lib";
    "@log" = "/var/log";
    "@identity" = "/etc/ssh";
    "@snapshots" = "/.snapshots";
  };
in {
  config = lib.mkIf cfg.enable {
    disko.devices.disk.system.content.partitions.root.content = {
      type = "btrfs";
      subvolumes =
        lib.mapAttrs (_: mountpoint: {
          inherit mountpoint;
          mountOptions = ["compress=zstd:${toString cfg.storage.compressionLevel}" "noatime"];
        })
        mounts;
    };
    fileSystems = lib.genAttrs ["/nix" "/var/lib" "/etc/ssh"] (_: {neededForBoot = true;});

    services.btrfs.autoScrub = {
      enable = lib.mkDefault true;
      interval = lib.mkDefault "weekly";
      # All six subvolumes share one filesystem: scrub it once.
      fileSystems = ["/"];
      limit = lib.mkDefault "32M";
    };
    services.snapper = lib.mkIf cfg.storage.snapshots.enable {
      snapshotInterval = "daily";
      cleanupInterval = "1d";
      persistentTimer = true;
      configs.root = {
        SUBVOLUME = "/";
        FSTYPE = "btrfs";
        TIMELINE_CREATE = true;
        TIMELINE_CLEANUP = true;
        TIMELINE_LIMIT_HOURLY = 0;
        TIMELINE_LIMIT_DAILY = cfg.storage.snapshots.dailyLimit;
        TIMELINE_LIMIT_WEEKLY = 0;
        TIMELINE_LIMIT_MONTHLY = 0;
        TIMELINE_LIMIT_QUARTERLY = 0;
        TIMELINE_LIMIT_YEARLY = 0;
        NUMBER_CLEANUP = true;
        NUMBER_LIMIT = cfg.storage.snapshots.numberLimit;
        NUMBER_LIMIT_IMPORTANT = cfg.storage.snapshots.numberLimit;
      };
    };
    # systemd's vendor tmpfiles rules use q/Q for these directories. That would
    # create implicit nested subvolumes which root snapshots replace with
    # unwritable inode-2 placeholders (including PrivateTmp's /tmp and /var/tmp).
    # NixOS writes these rules to 00-nixos.conf, before the vendor files. Only
    # the explicitly mounted Disko subvolumes should define recovery boundaries.
    systemd.tmpfiles.rules =
      [
        "d /.snapshots 0700 root root - -"
        "d /home 0755 - - -"
        "d /srv 0755 - - -"
        "d /var 0755 - - -"
        "d /var/tmp 1777 root root 30d"
      ]
      ++ lib.optional (!config.boot.tmp.cleanOnBoot) "d /tmp 1777 root root 10d";
  };
}
