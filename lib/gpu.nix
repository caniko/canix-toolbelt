# GPU vendor record helpers.
#
# Normalizes a `{igpu?, dgpu?}` record (vendors from `{"amd", "intel",
# "nvidia"}`) into a uniform structure usable by NixOS modules, Home Manager
# modules, and pkgs builders. Also tracks which GPU drives the display
# (displayGpu) so downstream consumers (iGPU offload, Chromium GPU flags)
# can conditionally skip DRI_PRIME wrapping when the iGPU is headless.
#
# Example:
#
#   let gpu = canix-toolbelt.lib.gpu.normalize {
#         igpu = "amd"; dgpu = "nvidia"; deviceType = "laptop";
#       };
#   in gpu.has.nvidia       # => true
#      gpu.mainGpu          # => "nvidia"  (dgpu wins over igpu)
#      gpu.displayGpu       # => "igpu"    (laptops display through iGPU)
#      gpu.igpuHasDisplay   # => true
#      gpu.isHybrid         # => true
#      gpu.vendors          # => ["amd" "nvidia"]
#
# The legacy aliases (`mainGpu`, `hasAmd`, etc.) are kept for backward
# compatibility with consumers that haven't moved to `main`/`has.<vendor>`.
let
  vendors = ["amd" "intel" "nvidia"];

  normalize = gpuData: let
    igpu = gpuData.igpu or null;
    dgpu = gpuData.dgpu or null;
    deviceType = gpuData.deviceType or null;
    displayGpu = gpuData.displayGpu or (
      if deviceType == "laptop"
      then "igpu"
      else if dgpu != null
      then "dgpu"
      else igpu
    );
    main =
      if dgpu != null
      then dgpu
      else igpu;
    has = vendor: igpu == vendor || dgpu == vendor;
  in {
    inherit dgpu igpu main deviceType displayGpu;
    isHybrid = igpu != null && dgpu != null;
    vendors = builtins.filter has vendors;
    has = {
      amd = has "amd";
      intel = has "intel";
      nvidia = has "nvidia";
    };
    igpuHasDisplay = igpu != null && displayGpu == "igpu";
    dgpuHasDisplay = dgpu != null && displayGpu == "dgpu";

    # Legacy aliases.
    mainGpu = main;
    hasAmd = has "amd";
    hasIntel = has "intel";
    hasNvidia = has "nvidia";
  };
in {
  inherit normalize vendors;

  noGpu = normalize {};

  forHost = hostsData: hostname:
    normalize (
      (hostsData.${hostname}.gpu or {})
      // {
        inherit ((hostsData.${hostname} or {})) deviceType;
      }
    );

  forHosts = hostsData:
    builtins.mapAttrs (_: hostData:
      normalize (
        (hostData.gpu or {})
        // {
          inherit (hostData) deviceType;
        }
      )
    ) hostsData;
}
