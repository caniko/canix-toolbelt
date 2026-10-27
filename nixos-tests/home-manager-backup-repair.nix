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
    if ${pkgs.lib.getExe repair} /tmp; then exit 1; fi
    touch "$out"
  ''
