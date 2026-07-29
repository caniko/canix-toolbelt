{
  inputs,
  pkgs,
}: let
  wrapper = import ../lib/chromiumGpu.nix {
    lib = pkgs.lib;
    wrapper-manager = inputs.wrapper-manager;
  };
  fakePackage = pkgs.symlinkJoin {
    name = "fake-chromium";
    paths = [
      (pkgs.writeShellScriptBin "fake-chromium" ''
        printf 'ENV=%s\n' "$TEST_GPU_ENV"
        printf '%s\n' "$@"
      '')
      (pkgs.writeShellScriptBin "fake-cli" ''
        printf '%s\n' "$@"
      '')
    ];
  };
  mkWrapped = args:
    wrapper ({
        inherit pkgs;
        basePackage = fakePackage;
        extraEnv.TEST_GPU_ENV.value = "angle";
      }
      // args);
  auto = mkWrapped {};
  wayland = mkWrapped {ozonePlatform = "wayland";};
  x11 = mkWrapped {ozonePlatform = "x11";};
  skipped = mkWrapped {skipPrograms = ["fake-cli"];};
  nonOverridable = wrapper {
    inherit pkgs;
    basePackage = removeAttrs fakePackage ["override"];
  };
in
  pkgs.runCommand "chromium-gpu-eval" {
    nativeBuildInputs = [pkgs.binutils pkgs.gnugrep];
  } ''
    set -euo pipefail

    ${auto}/bin/fake-chromium > auto.out
    grep -F -- 'ENV=angle' auto.out >/dev/null
    grep -F -- '--ozone-platform-hint=auto' auto.out >/dev/null
    grep -F -- '--use-gl=angle' auto.out >/dev/null
    if grep -F -- '--use-gl=egl' auto.out >/dev/null; then
      echo 'the generic Chromium wrapper must not inject --use-gl=egl' >&2
      exit 1
    fi

    ${wayland}/bin/fake-chromium > wayland.out
    grep -F -- '--enable-features=UseOzonePlatform' wayland.out >/dev/null
    grep -F -- '--ozone-platform=wayland' wayland.out >/dev/null
    ${x11}/bin/fake-chromium > x11.out
    grep -F -- '--enable-features=UseOzonePlatform' x11.out >/dev/null
    grep -F -- '--ozone-platform=x11' x11.out >/dev/null

    test -x ${skipped}/bin/fake-chromium
    test -x ${skipped}/bin/fake-cli
    test -x ${nonOverridable}/bin/fake-chromium
    ${skipped}/bin/fake-cli > skipped.out
    if grep -F -- '--use-gl=angle' skipped.out >/dev/null; then
      echo 'skipPrograms must leave the selected program unwrapped' >&2
      exit 1
    fi

    touch "$out"
  ''
