# GPU vendor record helpers.
#
# Normalizes a `{igpu?, dgpu?}` record (vendors from `{"amd", "intel",
# "nvidia"}`) into a uniform structure usable by NixOS modules, Home Manager
# modules, and pkgs builders.
#
# This is *inventory data only*: it picks package sets and answers "which GPUs
# exist". It does not pick the active renderer (that is the compositor's job on
# Wayland) and it does not carry the media-decode route (that is
# canix-toolbelt.gpuMedia). The legacy aliases (`mainGpu`, `hasAmd`, etc.) are
# kept for backward compatibility with consumers that haven't moved to
# `main`/`has.<vendor>`.
let
  vendors = ["amd" "intel" "nvidia"];

  normalize = gpuData: let
    igpu = gpuData.igpu or null;
    dgpu = gpuData.dgpu or null;
    deviceType = gpuData.deviceType or null;
    main =
      if dgpu != null
      then dgpu
      else igpu;
    has = vendor: igpu == vendor || dgpu == vendor;
  in {
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
