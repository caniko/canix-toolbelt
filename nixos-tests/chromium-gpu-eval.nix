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
        printf 'LIBVA=%s\n' "$LIBVA_DRIVER_NAME"
        printf 'NVD=%s\n' "$NVD_BACKEND"
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
  amdMedia = {
    enable = true;
    vendor = "amd";
    renderNode = "/dev/dri/by-path/pci-0000:65:00.0-render";
    libvaDriver = "radeonsi";
  };
  amd = mkWrapped {gpuMedia = amdMedia;};
  nvidia = mkWrapped {
    gpuMedia = {
      enable = true;
      vendor = "nvidia";
      renderNode = "/dev/dri/by-path/pci-0000:01:00.0-render";
      libvaDriver = "nvidia";
    };
  };
  overridden = mkWrapped {
    gpuMedia = amdMedia;
    extraEnv.LIBVA_DRIVER_NAME.value = "custom";
    extraFeatures = ["MyFeature"];
  };
in
  pkgs.runCommand "chromium-gpu-eval" {
    nativeBuildInputs = [pkgs.binutils pkgs.gnugrep pkgs.coreutils];
  } ''
    set -euo pipefail

    feature_count() {
      grep -o -- '--enable-features=[^ ]*' "$1" | wc -l
    }

    ${auto}/bin/fake-chromium > auto.out
    grep -F -- 'ENV=angle' auto.out >/dev/null
    grep -F -- '--ozone-platform-hint=auto' auto.out >/dev/null
    grep -F -- '--use-gl=angle' auto.out >/dev/null
    if grep -F -- '--use-gl=egl' auto.out >/dev/null; then
      echo 'the generic Chromium wrapper must not inject --use-gl=egl' >&2
      exit 1
    fi
    if grep -F -- '--render-node-override' auto.out >/dev/null; then
      echo 'disabled gpuMedia must not add a render-node override' >&2
      exit 1
    fi
    test "$(feature_count auto.out)" = 0

    ${wayland}/bin/fake-chromium > wayland.out
    grep -F -- '--ozone-platform=wayland' wayland.out >/dev/null
    if grep -F -- 'UseOzonePlatform' wayland.out >/dev/null; then
      echo 'explicit ozone must not add UseOzonePlatform to the feature switch' >&2
      exit 1
    fi
    ${x11}/bin/fake-chromium > x11.out
    grep -F -- '--ozone-platform=x11' x11.out >/dev/null
    test "$(feature_count wayland.out)" = 0
    test "$(feature_count x11.out)" = 0

    ${amd}/bin/fake-chromium > amd.out
    grep -F -- '--render-node-override=/dev/dri/by-path/pci-0000:65:00.0-render' amd.out >/dev/null
    grep -F -- 'LIBVA=radeonsi' amd.out >/dev/null
    grep -F -- '--enable-features=AcceleratedVideoEncoder,AcceleratedVideoDecodeLinuxZeroCopyGL,VaapiVideoEncoder,VaapiVideoDecoder,VaapiVideoDecodeLinuxGL,VaapiIgnoreDriverChecks' amd.out >/dev/null
    test "$(feature_count amd.out)" = 1
    if grep -F -- 'DRI_PRIME' amd.out >/dev/null; then
      echo 'no wrapper may set DRI_PRIME' >&2
      exit 1
    fi

    ${nvidia}/bin/fake-chromium > nvidia.out
    grep -F -- 'VaapiOnNvidiaGPUs' nvidia.out >/dev/null
    grep -F -- 'NVD=direct' nvidia.out >/dev/null
    grep -F -- 'LIBVA=nvidia' nvidia.out >/dev/null
    test "$(feature_count nvidia.out)" = 1

    ${overridden}/bin/fake-chromium > overridden.out
    grep -F -- 'LIBVA=custom' overridden.out >/dev/null
    grep -F -- 'MyFeature' overridden.out >/dev/null
    test "$(feature_count overridden.out)" = 1

    test -x ${skipped}/bin/fake-chromium
    test -x ${skipped}/bin/fake-cli
    ${skipped}/bin/fake-cli > skipped.out
    if grep -F -- '--use-gl=angle' skipped.out >/dev/null; then
      echo 'skipPrograms must leave the selected program unwrapped' >&2
      exit 1
    fi
    test -x ${nonOverridable}/bin/fake-chromium

    touch "$out"
  ''
