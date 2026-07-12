{lib}: {
  # Return a Home Manager activation DAG entry that replaces read-only HM
  # symlinks with writable copies. The source remains authoritative in the
  # generated generation; the copy exists only to let an application mutate
  # its runtime file after activation.
  mkWritableSymlinkCopies = {
    dag,
    pkgs,
    name,
    targets,
    mode ? "0644",
  }:
    dag.entryAfter ["writeBoundary"] ''
      for target in ${lib.concatStringsSep " " (map lib.escapeShellArg targets)}; do
        if [ -L "$target" ]; then
          tmp="$target.canix-copy"
          ${pkgs.coreutils}/bin/install -m ${lib.escapeShellArg mode} "$(realpath "$target")" "$tmp"
          ${pkgs.coreutils}/bin/mv -f "$tmp" "$target"
        fi
      done
    '';
}
