# Runs the repair against a fixture toplevel for the sandbox build user, whose
# passwd home is the writable build directory.
{pkgs}: let
  repair = import ../nix/home-manager-backup-repair.nix {
    inherit pkgs;
    tag = "fixture";
  };
  generation = pkgs.runCommand "fixture-home-manager-generation" {} ''
    mkdir -p "$out/home-files/.config"
    echo managed > "$out/home-files/.config/app.conf"
    echo managed > "$out/home-files/.profile"
    echo managed > "$out/home-files/.config/own.conf"
  '';
  ownFiles = pkgs.runCommand "fixture-home-manager-files" {} ''
    mkdir -p "$out" && echo managed > "$out/own"
  '';
  mkToplevel = name: unit:
    pkgs.runCommand "fixture-nixos-system-${name}" {} ''
      mkdir -p "$out/etc/systemd/system"
      cat > "$out/etc/systemd/system/${unit}" <<UNIT
      [Service]
      ExecStart=/bin/activate ${generation}
      UNIT
    '';
  # No backup extension configured: .bak files are not Home Manager's.
  noExtTop = mkToplevel "no-ext" "home-manager-nixbld.service";
  # Escaped unit name: \x62 is "b", so the user is still nixbld.
  escapedTop = pkgs.runCommand "fixture-nixos-system-escaped" {} ''
    mkdir -p "$out/etc/systemd/system"
    cat > "$out/etc/systemd/system/home-manager-nixbld.service" <<UNIT
    [Service]
    UNIT
    cat > "$out/etc/systemd/system/home-manager-nix\\x62ld.service" <<UNIT
    [Service]
    Environment="HOME_MANAGER_BACKUP_EXT=esc"
    ExecStart=/bin/activate ${generation}
    UNIT
  '';
  toplevel = pkgs.runCommand "fixture-nixos-system-test" {} ''
    mkdir -p "$out/etc/systemd/system"
    cat > "$out/etc/systemd/system/home-manager-nixbld.service" <<UNIT
    [Service]
    Environment="HOME_MANAGER_BACKUP_EXT=hm-bak"
    ExecStart=/bin/activate ${generation}
    UNIT
  '';
in
  pkgs.runCommand "home-manager-backup-repair" {nativeBuildInputs = [pkgs.getent];} ''
    test "$(id -un)" = nixbld
    home=$(getent passwd nixbld | cut -d: -f6)
    mkdir -p "$home/.config"
    echo live > "$home/.config/app.conf"
    echo stale > "$home/.config/app.conf.hm-bak"
    echo stale > "$home/.profile.hm-bak" # no live target: kept
    ${pkgs.lib.getExe repair} ${toplevel}
    test ! -e "$home/.config/app.conf.hm-bak"
    test "$(cat "$home"/.config/app.conf.hm-bak.fixture-*)" = stale
    test -e "$home/.profile.hm-bak"
    ln -s ${ownFiles}/own "$home/.config/own.conf"
    echo keep > "$home/.config/own.conf.hm-bak" # HM's own link: kept
    echo keep > "$home/.config/app.conf.bak"
    ${pkgs.lib.getExe repair} ${noExtTop}
    test -e "$home/.config/app.conf.bak"
    test -e "$home/.config/own.conf.hm-bak"
    echo stale > "$home/.config/app.conf.esc"
    ${pkgs.lib.getExe repair} ${escapedTop}
    test ! -e "$home/.config/app.conf.esc"
    test -e "$home/.config/own.conf.hm-bak"
    if ${pkgs.lib.getExe repair} /tmp; then exit 1; fi
    touch "$out"
  ''
