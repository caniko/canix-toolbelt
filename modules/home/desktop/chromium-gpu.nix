# Chromium GPU-acceleration wrapper (home-manager).
#
# Exposes `canix-toolbelt.chromiumGpu.wrap`, which wraps a Chromium-based
# package with VA-API video accel feature flags + the matching env. Decode
# follows canix-toolbelt.igpu: when igpu.decodeActive is set (battery/travel),
# VA-API decode is pinned to the iGPU via --render-node-override +
# LIBVA_DRIVER_NAME; otherwise it follows the rendering GPU.
#
# `wrapper-manager` is injected by the flake (homeModules) so consumers need
# not thread `inputs.wrapper-manager` themselves.
{wrapper-manager}: {
  config,
  lib,
  pkgs,
  gpu ? null,
  igpu ? null,
  dgpu ? null,
  ...
}: let
  igpuCfg = config.canix-toolbelt.igpu;
  gpuInfo =
    if gpu != null
    then gpu
    else {
      inherit dgpu igpu;
    };

  # Match the host's primary GPU; DRI_PRIME from canix-toolbelt.igpu still
  # controls hybrid routing, but Chromium's vendor-specific VA-API flags follow
  # the GPU selected by the shared host registry.
  renderingGpu =
    gpuInfo.main or (gpuInfo.mainGpu or (
      if gpuInfo.dgpu != null
      then gpuInfo.dgpu
      else if gpuInfo.igpu != null
      then gpuInfo.igpu
      else null
    ));

  isIntel = renderingGpu == "intel";

  # VA-API *decode* vendor. This drives only the decode feature flags + libva
  # env (NOT the rendering/ANGLE backend, which is GL on every host). When
  # offloading decode (battery/travel) it is the iGPU. Otherwise, on a hybrid
  # host the session renders *and* decodes on the iGPU (e.g. a PRIME
  # render-offload laptop whose dGPU is parked), so follow the iGPU vendor
  # rather than the dGPU that `renderingGpu` resolves to; single-GPU hosts fall
  # back to the rendering GPU.
  decodeActive = igpuCfg.decodeActive or false;
  decodeVendor =
    if decodeActive
    then igpuCfg.type
    else if igpuCfg.enable && igpuCfg.type != null
    then igpuCfg.type
    else renderingGpu;
  isAmd = decodeVendor == "amd";
  isNvidia = decodeVendor == "nvidia";

  # Only route GL/Vulkan rendering through DRI_PRIME when the iGPU drives
  # a display. A headless iGPU cannot create an on-screen GL context; trying
  # crashes the GPU process (SIGTRAP in Electron/Chromium). Decode VA-API
  # flags below use renderNode directly and don't need DRI_PRIME.
  chromiumEnv =
    lib.optionalAttrs (igpuCfg.enable && igpuCfg.igpuHasDisplay) {
      DRI_PRIME.value = igpuCfg.driPrimeValue;
    }
    // (
      if decodeActive
      then {LIBVA_DRIVER_NAME.value = igpuCfg.decodeDriver;}
      else
        lib.optionalAttrs isNvidia {
          LIBVA_DRIVER_NAME.value = "nvidia";
          NVD_BACKEND.value = "direct";
        }
    );

  chromiumFlags = {enableAngleVulkan ? true}:
    [
      "--password-store=gnome-libsecret"
      # Force native Wayland Ozone instead of Xwayland. `auto` self-detects:
      # Wayland when WAYLAND_DISPLAY is set (every consuming host runs a Wayland
      # compositor), X11 otherwise. Under Xwayland, ANGLE-Vulkan fails to find
      # an EGL config on hybrid GPUs and the whole GPU process dies ("No
      # suitable EGL configs found"); native Wayland avoids that entirely.
      "--ozone-platform-hint=auto"
      "--ignore-gpu-blocklist"
      "--disable-gpu-driver-bug-workaround"
      "--enable-unsafe-webgpu"
      # ANGLE on its GL backend. ANGLE-Vulkan is deliberately NOT used: under
      # Wayland the Vulkan path can't back the compositor surface (Chromium
      # logs "not compatible with Vulkan" and falls back to GL anyway), and on
      # multi-GPU hosts without a MESA_VK_DEVICE_SELECT pin it cannot choose a
      # device, killing the GPU process.
      "--use-gl=angle"
    ]
    # Pin VA-API decode to the iGPU render node; Chromium otherwise hardcodes
    # /dev/dri/renderD128. Paired with LIBVA_DRIVER_NAME in chromiumEnv.
    ++ lib.optional decodeActive "--render-node-override=${igpuCfg.renderNode}"
    ++ [
      "--enable-features=${
        lib.concatStringsSep "," (
          [
            "AcceleratedVideoEncoder"
            "AcceleratedVideoDecodeLinuxZeroCopyGL"
            ## Codecs
            ### VAAPI: https://chromium.googlesource.com/chromium/src/+/master/docs/hardware/gpu/vaapi.md#VaAPI-on-Linux-with-Vulkan
            "VaapiVideoEncoder"
            "VaapiVideoDecoder"
            "VaapiVideoDecodeLinuxGL"
            "VaapiIgnoreDriverChecks"
          ]
          # NVIDIA: VA-API bridge via nvidia-vaapi-driver
          ++ lib.optional isNvidia "VaapiOnNvidiaGPUs"
        )
      }"
    ];

  mkChromiumGpuWrapper = args: let
    wrapperArgs =
      if lib.isDerivation args
      then {basePackage = args;}
      else args;
    inherit (wrapperArgs) basePackage;
    wrapperName = wrapperArgs.wrapperName or (lib.getName basePackage);
    enableAngleVulkan = wrapperArgs.enableAngleVulkan or true;
    skipPrograms = wrapperArgs.skipPrograms or [];
    wrapperBasePackage =
      if basePackage ? overrideAttrs
      then
        basePackage.overrideAttrs (old: {
          # wrapper-manager copies basePackage.meta onto the generated
          # symlinkJoin. Keep license enforcement on the real package, not on
          # the local wrapper derivation.
          meta = removeAttrs (old.meta or {}) ["license" "sourceProvenance"];
        })
      else basePackage;
    inherit
      ((wrapper-manager.lib {
          inherit pkgs;
          modules = [
            {
              wrappers.${wrapperName} = {
                basePackage = wrapperBasePackage;
                prependFlags = chromiumFlags {inherit enableAngleVulkan;};
                env = chromiumEnv;
                programs = lib.genAttrs skipPrograms (_: {});
              };
            }
          ];
        }).config.wrappers.${
          wrapperName
        })
      wrapped
      ;
  in
    wrapped
    // {
      override = f:
        mkChromiumGpuWrapper (
          wrapperArgs
          // {
            basePackage = basePackage.override f;
          }
        );
    };
in {
  options.canix-toolbelt.chromiumGpu.wrap = lib.mkOption {
    type = lib.types.raw;
    readOnly = true;
    description = "Wrap a Chromium-based package with canix-toolbelt GPU acceleration flags and environment.";
  };

  config.canix-toolbelt.chromiumGpu.wrap = mkChromiumGpuWrapper;
}
