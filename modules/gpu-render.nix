# Optional game/3D device route. Rendering defaults to the desktop when disabled.
{fleetixGpu}: args @ {
  config,
  lib,
  ...
}: let
  gpuRender = import ../lib/gpu-route.nix {
    role = "render";
    inherit args config;
  };
  cfg = config.canix-toolbelt.gpuRender;
in {
  options.canix-toolbelt.gpuRender = {
    enable = lib.mkEnableOption "the explicit game/3D GPU route" // {default = gpuRender.enable or false;};
    renderNode = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = gpuRender.renderNode or null;
      description = "Stable /dev/dri/by-path/pci-...-render alias. Consumers validate device identity and supported runtime drivers.";
    };
  };
  config.assertions = [
    {
      assertion = !cfg.enable || fleetixGpu.validRenderNode cfg.renderNode;
      message = "canix-toolbelt.gpuRender: enabled routing requires a stable PCI render-node alias";
    }
  ];
}
