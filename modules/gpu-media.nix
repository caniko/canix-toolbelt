# Explicit GPU media-decode route (shared by the NixOS and home-manager exports).
#
# Replaces the old canix-toolbelt.igpu decode contract. Whereas igpu conflated
# render offload (DRI_PRIME), display topology, and VA-API decode, this module
# carries only the *media route*: which GPU hardware video decode should use.
#
# Normal Wayland applications render on the compositor's GPU; they do not need
# this at all. Consumers with app-specific decode knobs (chromium-gpu, firefox,
# mpv) read this route and translate it into their own flags/env.
#
# The route has no opinion about power profiles or battery state — that policy
# belongs to the host, not to a reusable hardware module.
{
  config,
  lib,
  pkgs,
  gpuMedia ? null,
  ...
}: let
  cfg = config.canix-toolbelt.gpuMedia;
in {
  options.canix-toolbelt.gpuMedia = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = gpuMedia.enable or false;
      description = ''
        Whether applications should use the configured media-decode route.
      '';
    };
    vendor = lib.mkOption {
      type = lib.types.nullOr (lib.types.enum ["amd" "intel" "nvidia"]);
      default = gpuMedia.vendor or null;
      description = "Vendor of the GPU that hardware video decode should use; drives app-specific flags.";
    };
    renderNode = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = gpuMedia.renderNode or null;
      example = "/dev/dri/by-path/pci-0000:65:00.0-render";
      description = ''
        Stable by-path DRM render node of the media GPU. Prefer the by-path
        node over a bare renderD12N number, which is assigned in probe order
        and can renumber across boots.

        Note for Chromium-family consumers: --render-node-override also
        influences the GBM/presenting device, so the configured node must be
        safe for both rendering and decoding.
      '';
    };
    libvaDriver = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = gpuMedia.libvaDriver or null;
      description = ''
        libva driver name (LIBVA_DRIVER_NAME) matching the media GPU:
        radeonsi (amd) / iHD (intel) / nvidia. Selects the driver
        implementation; decoding still needs the renderNode above.
      '';
    };
  };

  config = {
    assertions = [
      {
        assertion = !cfg.enable || (cfg.vendor != null && cfg.renderNode != null && cfg.libvaDriver != null);
        message = "canix-toolbelt.gpuMedia: enable requires vendor, renderNode, and libvaDriver to be set";
      }
      {
        assertion = cfg.renderNode == null || lib.hasPrefix "/dev/dri/" cfg.renderNode;
        message = "canix-toolbelt.gpuMedia.renderNode must be an absolute /dev/dri/... path";
      }
    ];
  };
}
