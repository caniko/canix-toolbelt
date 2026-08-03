# iGPU offload for hybrid-GPU hosts (home-manager).
#
# Three orthogonal mechanisms:
#
#   * `wrap`        — wraps a package's executables with DRI_PRIME so their
#                     OpenGL/Vulkan *rendering* runs on the integrated GPU.
#                     When the iGPU has no display (headless, e.g. a desktop
#                     whose dGPU drives the monitors), wrapping is skipped
#                     because on-screen GL contexts need a connected CRTC.
#   * `wrapDecode`  — wraps a package with LIBVA_DRIVER_NAME + render node
#                     for VA-API hardware video decode on the iGPU. Always
#                     applies regardless of display status.
#   * decode levers — `renderNode` / `decodeDriver` / `decodeActive` /
#                     `enableDecode` pin VA-API decode to the iGPU.
#                     DRI_PRIME does NOT move decode (it is GL/Vulkan only),
#                     so decode needs an explicit device + driver; consumers
#                     (e.g. an mpv module, the chromium-gpu module) read
#                     these to wire the actual app-specific knobs.
#
# Vendor/topology can be supplied either via a normalized `gpu` record
# (canix-toolbelt.lib.gpu.normalize) passed as a module argument, or by setting
# `type`/`enable`/`renderNode` directly. `igpu`/`dgpu` module args are optional
# conveniences for consumers that thread raw vendors instead of `gpu`.
{
  config,
  lib,
  pkgs,
  gpu ? null,
  igpu ? null,
  dgpu ? null,
  ...
}: let
  cfg = config.canix-toolbelt.igpu;
  gpuInfo =
    if gpu != null
    then gpu
    else {
      inherit dgpu igpu;
      deviceType = null;
      displayGpu = null;
      igpuHasDisplay = false;
    };
  wrapPkg = pkg: let
    wrapped = pkgs.symlinkJoin {
      name = "${pkg.pname or pkg.name}-igpu";
      paths = [pkg];
      nativeBuildInputs = [pkgs.makeWrapper];
      postBuild = ''
        for f in $out/bin/*; do
          if [ -f "$f" ] && [ -x "$f" ]; then
            wrapProgram "$f" --set DRI_PRIME "${cfg.driPrimeValue}"
          fi
        done
      '';
    };
  in
    wrapped
    // {
      pname = pkg.pname or pkg.name;
      version = pkg.version or "";
      meta = pkg.meta or {};
      passthru = pkg.passthru or {};
      override =
        lib.setFunctionArgs
        (args: wrapPkg (pkg.override args))
        (lib.functionArgs pkg.override);
    };
  # Wrap a package with LIBVA_DRIVER_NAME pinned to the iGPU's VA-API
  # driver. This helps GStreamer, ffmpeg, and other libva consumers
  # route hardware video decode to the iGPU. Browser-level decode
  # flags (--render-node-override, etc.) come from chromiumGpu.wrap.
  wrapPkgDecode = pkg: let
    wrapped = pkgs.symlinkJoin {
      name = "${pkg.pname or pkg.name}-igpu-decode";
      paths = [pkg];
      nativeBuildInputs = [pkgs.makeWrapper];
      postBuild = ''
        for f in $out/bin/*; do
          if [ -f "$f" ] && [ -x "$f" ]; then
            wrapProgram "$f" --set LIBVA_DRIVER_NAME "${cfg.decodeDriver}"
          fi
        done
      '';
    };
  in
    wrapped
    // {
      pname = pkg.pname or pkg.name;
      version = pkg.version or "";
      meta = pkg.meta or {};
      passthru = pkg.passthru or {};
      override =
        lib.setFunctionArgs
        (args: wrapPkgDecode (pkg.override args))
        (lib.functionArgs pkg.override);
    };
in {
  options.canix-toolbelt.igpu = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = gpuInfo.igpu != null && gpuInfo.dgpu != null;
      description = "Whether to enable iGPU offloading for desktop apps. Auto-enabled on hybrid GPU hosts.";
    };
    type = lib.mkOption {
      type = lib.types.nullOr (lib.types.enum ["amd" "intel" "nvidia"]);
      default = gpuInfo.igpu;
      description = "GPU vendor of the iGPU that desktop apps render on";
    };
    driPrimeValue = lib.mkOption {
      type = lib.types.str;
      default = "1";
      description = "DRI_PRIME value to route rendering to iGPU";
    };
    renderNode = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "/dev/dri/by-path/pci-0000:65:00.0-render";
      description = ''
        Stable by-path DRM render node of the iGPU, used to pin VA-API
        hardware video *decode* to the iGPU. DRI_PRIME (see `wrap`) only
        moves OpenGL/Vulkan *rendering* — it does not relocate VA-API decode
        — so decode pinning needs an explicit device. Prefer the by-path
        node over a bare renderD12N number, which is assigned in probe order
        and can renumber across boots. Leave null to leave decode on its
        default device.
      '';
    };
    decodeDriver = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      readOnly = true;
      description = ''
        libva driver name (LIBVA_DRIVER_NAME) matching the iGPU vendor,
        derived from `type`: radeonsi (amd) / iHD (intel) / nvidia.
      '';
    };
    enableDecode = lib.mkOption {
      type = lib.types.bool;
      default = gpuInfo.deviceType != "laptop";
      description = ''
        Whether to pin VA-API hardware video decode to the iGPU at all
        times. True by default on desktops and servers (always-on iGPU
        decode), false on laptops (only offload decode on battery/travel
        to save power).
      '';
    };
    decodeActive = lib.mkOption {
      type = lib.types.bool;
      readOnly = true;
      description = ''
        Whether desktop media apps should route VA-API hardware video decode
        to the iGPU right now. True when decode is beneficial: on battery
        (the `travel` profile, keeping the discrete GPU asleep) or when
        `enableDecode` is true (desktops where the iGPU is always available).
      '';
    };
    igpuHasDisplay = lib.mkOption {
      type = lib.types.bool;
      readOnly = true;
      default = gpuInfo.igpuHasDisplay or false;
      description = ''
        Whether the integrated GPU drives a display (i.e. it has a connected
        CRTC). When false, DRI_PRIME wrapping is skipped for display-needing
        applications to avoid GPU process crashes on headless iGPUs. Decode
        and compute offload are unaffected.
      '';
    };
    wrap = lib.mkOption {
      type = lib.types.raw;
      readOnly = true;
      description = ''
        Wrap a package with DRI_PRIME for iGPU rendering offload. Skips
        wrapping when the iGPU has no display (headless iGPU), as on-screen
        rendering requires a connected CRTC. Use `wrapDecode` for VA-API
        decode offload (no display needed).
      '';
    };
    wrapDecode = lib.mkOption {
      type = lib.types.raw;
      readOnly = true;
      description = ''
        Wrap a package with LIBVA_DRIVER_NAME pinned to the iGPU's VA-API
        driver for hardware video decode. Always applies when decodeDriver
        is configured, regardless of iGPU display status. Intended for
        media players and transcoders that don't already route decode
        through chromium-gpu.
      '';
    };
  };

  config = {
    assertions = [
      {
        assertion = !cfg.enable || cfg.type != null;
        message = "canix-toolbelt.igpu.type must be set when canix-toolbelt.igpu.enable is true";
      }
    ];

    canix-toolbelt.igpu.wrap = pkg:
      if cfg.enable && cfg.igpuHasDisplay
      then wrapPkg pkg
      else pkg;

    canix-toolbelt.igpu.wrapDecode = pkg:
      if cfg.enable && cfg.renderNode != null && cfg.decodeDriver != null
      then wrapPkgDecode pkg
      else pkg;

    canix-toolbelt.igpu.decodeDriver =
      if cfg.type == "amd"
      then "radeonsi"
      else if cfg.type == "intel"
      then "iHD"
      else if cfg.type == "nvidia"
      then "nvidia"
      else null;

    canix-toolbelt.igpu.decodeActive =
      cfg.enable
      && cfg.renderNode != null
      && cfg.decodeDriver != null
      && (config.canix-toolbelt.profiles.travel.enable or false || cfg.enableDecode);

    home.shellAliases = lib.mkIf cfg.enable {
      igpu-run = "DRI_PRIME=${cfg.driPrimeValue}";
    };
  };
}
