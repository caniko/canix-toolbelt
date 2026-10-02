{
  pkgs,
  inputs,
}: let
  inherit (pkgs) lib;
  gpu = inputs.fleetix.lib.gpu;
  fleetixGpu = gpu;
  evaluate = modules:
    (lib.evalModules {
      modules =
        [
          (import ../modules/home/gaming/modde-gpu.nix {inherit fleetixGpu;})
          ({lib, ...}: {
            options.assertions = lib.mkOption {
              type = lib.types.listOf lib.types.attrs;
              default = [];
            };
            options.programs.modde.gpu.renderNode = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
            };
          })
        ]
        ++ modules;
    }).config;
  route = "/dev/dri/by-path/pci-0000:03:00.0-render";
  valid = evaluate [
    {
      canix-toolbelt.gpuRender = {
        enable = true;
        renderNode = route;
      };
    }
  ];
  default = evaluate [];
  topologyDefault =
    (lib.evalModules {
      specialArgs.gpuRender = {
        enable = true;
        renderNode = route;
      };
      modules = [
        (import ../modules/gpu-render.nix {inherit fleetixGpu;})
        ({lib, ...}: {
          options.assertions = lib.mkOption {
            type = lib.types.listOf lib.types.attrs;
            default = [];
          };
        })
      ];
    }).config;
  override = evaluate [
    {
      canix-toolbelt.gpuRender = {
        enable = true;
        renderNode = route;
      };
      programs.modde.gpu.renderNode = "/dev/dri/by-path/pci-0000:65:00.0-render";
    }
  ];
  invalid = evaluate [
    {
      canix-toolbelt.gpuRender = {
        enable = true;
        renderNode = "/dev/dri/renderD128";
      };
    }
  ];
  oldModde =
    (lib.evalModules {
      modules = [
        (import ../modules/home/gaming/modde-gpu.nix {inherit fleetixGpu;})
        ({lib, ...}: {
          options.assertions = lib.mkOption {
            type = lib.types.listOf lib.types.attrs;
            default = [];
          };
        })
        {
          canix-toolbelt.gpuRender = {
            enable = true;
            renderNode = route;
          };
        }
      ];
    }).config;
in
  assert (gpu.normalize {render.renderNode = route;}).render.renderNode == route;
  assert gpu.noGpu.render == null;
  assert !(builtins.tryEval (builtins.deepSeq (gpu.normalize {render.renderNode = "/dev/dri/renderD128";}).render true)).success;
  assert valid.programs.modde.gpu.renderNode == route;
  assert lib.all (a: a.assertion) valid.assertions;
  assert default.programs.modde.gpu.renderNode == null;
  assert topologyDefault.canix-toolbelt.gpuRender.enable;
  assert topologyDefault.canix-toolbelt.gpuRender.renderNode == route;
  assert lib.all (a: a.assertion) topologyDefault.assertions;
  assert override.programs.modde.gpu.renderNode != route;
  assert lib.any (a: !a.assertion) invalid.assertions;
  assert lib.any (a: !a.assertion) oldModde.assertions;
    pkgs.writeText "gpu-render-eval" "ok"
