# GPU vendor record helpers.
#
# Normalizes a `{igpu?, dgpu?}` record (vendors from `{"amd", "intel",
# "nvidia"}`) into a uniform structure usable by NixOS modules, Home Manager
# modules, and pkgs builders.
#
# This carries inventory and an optional primary-GPU compute request for
# package selection. It does not pick the active renderer (the compositor's job on
# Wayland) and it does not carry the media-decode route (that is
# canix-toolbelt.gpuMedia). The legacy aliases (`mainGpu`, `hasAmd`, etc.) are
# kept for backward compatibility with consumers that haven't moved to
# `main`/`has.<vendor>`.
let
  vendors = ["amd" "intel" "nvidia"];
  computeVendors = {
    oneapi = "intel";
    rocm = "amd";
    cuda = "nvidia";
  };

  normalize = gpuData: let
    igpu = gpuData.igpu or null;
    dgpu = gpuData.dgpu or null;
    deviceType = gpuData.deviceType or null;
    main =
      if dgpu != null
      then dgpu
      else igpu;
    has = vendor: igpu == vendor || dgpu == vendor;
    compute = gpuData.compute or null;
    backend =
      if compute == null
      then null
      else compute.backend or null;
  in {
    # A declaration is an explicit request, never inferred from vendor alone.
    compute =
      if compute == null
      then null
      else if backend == null || !(builtins.hasAttr backend computeVendors)
      then throw "GPU compute: expected backend oneapi, rocm, or cuda"
      else if main != computeVendors.${backend}
      then throw "GPU compute: ${backend} requires a ${computeVendors.${backend}} primary GPU (dGPU, otherwise iGPU)"
      else {inherit backend;};
    inherit dgpu igpu main deviceType;
    isHybrid = igpu != null && dgpu != null;
    vendors = builtins.filter has vendors;
    has = {
      amd = has "amd";
      intel = has "intel";
      nvidia = has "nvidia";
    };

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
    builtins.mapAttrs (
      _: hostData:
        normalize (
          (hostData.gpu or {})
          // {
            inherit (hostData) deviceType;
          }
        )
    )
    hostsData;
}
