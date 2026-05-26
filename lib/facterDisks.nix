{disks}: let
  requireDiskByPath = byPathName: let
    devicePath = "/dev/disk/by-path/${byPathName}";
    matches =
      builtins.filter
      (disk: builtins.elem devicePath (disk.unix_device_names or []))
      disks;
    matchCount = builtins.length matches;
    matchIds = builtins.map (disk: disk.sysfs_id or "<unknown>") matches;
  in
    if matchCount == 1
    then devicePath
    else if matchCount == 0
    then throw "facter-disks: no disk exposes '${devicePath}' in config.facter.report.hardware.disk"
    else throw "facter-disks: expected exactly one disk for '${devicePath}' in config.facter.report.hardware.disk, found ${toString matchCount} (${builtins.concatStringsSep ", " matchIds})";
in {
  inherit requireDiskByPath;
}
