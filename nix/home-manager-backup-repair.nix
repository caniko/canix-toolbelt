# Move stale Home Manager backups aside before Home Manager activation.
#
# HM refuses to activate when both a managed target and its `<target>.<ext>`
# backup exist. For every home-manager-<user>.service in SYSTEM_CONFIG, this
# renames each colliding backup to `<backup>.<tag>-<UTC stamp>[-N]`. Services
# without a backup extension, and targets that are already Home Manager's own
# symlinks, are not collisions and are left alone. `startAsUserService` has no
# system-level unit, so it is out of scope: nothing runs at NixOS activation.
{
  pkgs,
  tag ? "toolbelt",
}:
pkgs.writeShellApplication {
  name = "home-manager-backup-repair";
  runtimeInputs = [pkgs.coreutils pkgs.findutils pkgs.getent pkgs.gnused pkgs.systemd];
  text = ''
    if [ "$#" -ne 1 ]; then
      printf 'usage: %s SYSTEM_CONFIG\n' "$0" >&2
      exit 64
    fi

    candidate=$(readlink -f -- "$1")
    case "$candidate" in
      /nix/store/*-nixos-system-*) ;;
      *)
        printf 'not a NixOS system toplevel: %s\n' "$candidate" >&2
        exit 64
        ;;
    esac

    tag=${pkgs.lib.escapeShellArg tag}
    stamp=$(date -u +%Y%m%dT%H%M%SZ)
    for service in "$candidate/etc/systemd/system"/home-manager-*.service; do
      [ -f "$service" ] || continue
      user=''${service##*/home-manager-}
      user=$(systemd-escape --unescape -- "''${user%.service}")
      home=$(getent passwd "$user" | cut -d: -f6)
      [ -n "$home" ] || continue
      generation=$(sed -n 's|^ExecStart=[^ ]* \(.*-home-manager-generation\)$|\1|p' "$service")
      [ -n "$generation" ] || continue
      files_root=$(readlink -f "$generation/home-files")
      [ -d "$files_root" ] || continue
      backup_ext=$(sed -n 's|^Environment="HOME_MANAGER_BACKUP_EXT=\(.*\)"$|\1|p' "$service")
      [ -n "$backup_ext" ] || continue
      while IFS= read -r -d "" managed; do
        relative=''${managed#"$files_root/"}
        target="$home/$relative"
        backup="$target.$backup_ext"
        [ -e "$target" ] && [ -e "$backup" ] || continue
        case "$(readlink -- "$target" || true)" in
          /nix/store/*-home-manager-files/*) continue ;;
        esac
        destination="$backup.$tag-$stamp"
        index=0
        while [ -e "$destination" ]; do
          index=$((index + 1))
          destination="$backup.$tag-$stamp-$index"
        done
        mv -- "$backup" "$destination"
        printf '%s: moved stale Home Manager backup %s to %s\n' "$tag" "$backup" "$destination" >&2
      done < <(find "$files_root" \( -type f -o -type l \) -print0)
    done
  '';
}
