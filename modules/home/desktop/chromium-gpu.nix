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

  # VA-API *decode* vendor: the iGPU when offloading decode (battery/travel),
  # otherwise the rendering GPU. Rendering still follows DRI_PRIME / the host
  # registry; only the decode device + driver move.
  decodeActive = igpuCfg.decodeActive or false;
  decodeVendor =
    if decodeActive
    then igpuCfg.type
    else renderingGpu;
  isAmd = decodeVendor == "amd";
  isNvidia = decodeVendor == "nvidia";

  chromiumEnv =
    lib.optionalAttrs igpuCfg.enable {
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
      "--ignore-gpu-blocklist"
      "--disable-gpu-driver-bug-workaround"
      "--enable-unsafe-webgpu"
      "--use-gl=angle"
    ]
    ++ lib.optional (enableAngleVulkan && !isIntel) "--use-angle=vulkan"
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
          # AMD: Vulkan feature flags required for Mesa VA-API path
          ++ lib.optionals isAmd [
            "Vulkan"
            "DefaultANGLEVulkan"
            "VulkanFromANGLE"
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
    inherit
      ((wrapper-manager.lib {
          inherit pkgs;
          modules = [
            {
              wrappers.${wrapperName} = {
                inherit basePackage;
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
