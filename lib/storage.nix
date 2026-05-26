{
  # FreeDesktop trash dirs on a non-home filesystem so file managers can move
  # files cross-device without falling back to copy+delete. UID matches the
  # user's POSIX uid; the dir name format is mandated by the spec.
  #
  # mkFreedesktopTrash { user = "can"; uid = 1000; root = "/data"; }
  #   => systemd.tmpfiles.rules entries for /data/.Trash-1000/{,files,info}
  mkFreedesktopTrash = {
    user,
    uid ? 1000,
    root ? "/data",
  }: let
    base = "${root}/.Trash-${toString uid}";
  in [
    "d ${base} 0700 ${user} users -"
    "d ${base}/files 0700 ${user} users -"
    "d ${base}/info 0700 ${user} users -"
  ];

  # PAM-mount volume entry for fstab-style remote mounts that should appear
  # at login. The shape mirrors `pam_mount.conf.xml`'s <volume> element; the
  # helper just wraps the XML literal so callers don't hand-write attribute
  # quoting.
  mkPamMountVolume = {
    user,
    fstype,
    server,
    path,
    mountpoint,
    options ? "",
  }: ''<volume user="${user}" fstype="${fstype}" server="${server}" path="${path}" mountpoint="${mountpoint}" options="${options}" />'';

  # Schedule SSDs as 'none' rather than the kernel default 'mq-deadline'.
  # Lower latency for random IO; rotational drives are unaffected.
  ssdNoneSchedulerUdevRule = ''
    ACTION=="add|change", KERNEL=="sd[a-z]", ATTR{queue/rotational}=="0", ATTR{queue/scheduler}="none"
  '';
}
