# Import alongside modde's own Home Manager module. The application retains
# ownership of its launch schema and its per-installation overrides.
{fleetixGpu}: {
  config,
  lib,
  options,
  ...
}: let
  route = config.canix-toolbelt.gpuRender;
  moddeHasGpuRoute = options ? programs.modde.gpu.renderNode;
in {
  imports = [(import ../../gpu-render.nix {inherit fleetixGpu;})];
  config =
    {
      assertions = [
        {
          assertion = !route.enable || moddeHasGpuRoute;
          message = "modde-gpu: enabled routing requires a modde Home Manager module with programs.modde.gpu.renderNode";
        }
      ];
    }
    // lib.optionalAttrs moddeHasGpuRoute {
      programs.modde.gpu.renderNode = lib.mkDefault (
        if route.enable
        then route.renderNode
        else null
      );
    };
}
