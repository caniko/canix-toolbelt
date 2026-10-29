{lib, ...}: {
  services.btrfs.autoScrub = {
    enable = true;
    interval = lib.mkDefault "weekly";
  };
}
