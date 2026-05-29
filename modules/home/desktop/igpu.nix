# iGPU offload for hybrid-GPU hosts (home-manager).
#
# Two distinct mechanisms, both runtime (nothing here is a package-set choice):
#
#   * `wrap`        — wraps a package's executables with DRI_PRIME so their
#                     OpenGL/Vulkan *rendering* runs on the integrated GPU.
#   * decode levers — `renderNode` / `decodeDriver` / `decodeActive` pin
#                     VA-API hardware video *decode* to the iGPU. DRI_PRIME does
#                     NOT move decode (it is GL/Vulkan only), so decode needs an
#                     explicit device + driver; consumers (e.g. an mpv module,
#                     the chromium-gpu module) read these to wire the actual
#                     app-specific knobs (mpv --vaapi-device, Chromium
#                     --render-node-override, LIBVA_DRIVER_NAME).
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
      override = f: wrapPkg (pkg.override f);
    };
in {
  options.canix-toolbelt.igpu = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = gpuInfo.igpu != null && gpuInfo.dgpu != null;
      description = "Whether to enable iGPU offloading for desktop apps. Auto-enabled on hybrid GPU hosts.";
    };
    type = lib.mkOption {
      type = lib.types.nullOr (lib.types.enum ["amd" "nvidia" "intel"]);
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
    decodeActive = lib.mkOption {
      type = lib.types.bool;
      readOnly = true;
      description = ''
        Whether desktop media apps should route VA-API hardware video decode
        to the iGPU right now. True only when the offload is an actual win —
        i.e. on battery (the `travel` profile), where it keeps the discrete
        GPU asleep — and a `renderNode` is set. On hosts whose discrete GPU
        is always powered (no `travel` profile) this stays false and decode
        remains on the primary GPU.
      '';
    };
    wrap = lib.mkOption {
      type = lib.types.raw;
      readOnly = true;
      description = "Conditionally wrap a package with DRI_PRIME for iGPU offloading";
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
      if cfg.enable
      then wrapPkg pkg
      else pkg;

    canix-toolbelt.igpu.decodeDriver =
      if cfg.type == "amd"
      then "radeonsi"
      else if cfg.type == "intel"
      then "iHD"
      else if cfg.type == "nvidia"
      then "nvidia"
      else null;

    # Only offload decode when it pays off: on battery (travel), keeping the
    # dGPU parked. Hosts with no `travel` profile leave this false and decode
    # stays on their always-on primary GPU.
    canix-toolbelt.igpu.decodeActive =
      cfg.enable
      && (config.canix-toolbelt.profiles.travel.enable or false)
      && cfg.renderNode != null
      && cfg.decodeDriver != null;

    home.shellAliases = lib.mkIf cfg.enable {
      igpu-run = "DRI_PRIME=${cfg.driPrimeValue}";
    };
  };
}
