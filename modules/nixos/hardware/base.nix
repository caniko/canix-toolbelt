# Universal hardware basics: enable redistributable firmware. Most other
# modules in this flake key off `hardware.enableRedistributableFirmware`.
{lib, ...}: {
  hardware.enableRedistributableFirmware = lib.mkDefault true;
}
