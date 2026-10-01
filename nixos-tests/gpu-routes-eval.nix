{
  inputs,
  pkgs,
}: let
  inherit (pkgs) lib;
  fleetixGpu = inputs.fleetix.lib.gpu;
  renderNode = "/dev/dri/by-path/pci-0000:03:00.0-render";
  mediaNode = "/dev/dri/by-path/pci-0000:65:00.0-render";
  overrideNode = "/dev/dri/by-path/pci-0000:66:00.0-render";
  routes = fleetixGpu.routes {
    igpu = "intel";
    dgpu = "amd";
    render.renderNode = renderNode;
    media = {
      vendor = "intel";
      renderNode = mediaNode;
      libvaDriver = "iHD";
    };
    compute.backend = "rocm";
  };
  # Capture the real adapters' wrapper-manager arguments without realisation.
  wrapper-manager.lib = args: {
    config.wrappers =
      builtins.mapAttrs (_: wrapper: {
        wrapped = {
          testEnv = wrapper.env or {};
          testFlags = wrapper.prependFlags or [];
        };
      })
      (builtins.head args.modules).wrappers;
  };
  evaluate = specialArgs: extraModules:
    (lib.evalModules {
      specialArgs = {inherit pkgs;} // specialArgs;
      modules =
        [
          (import ../modules/gpu-media.nix {inherit fleetixGpu;})
          (import ../modules/home/gaming/modde-gpu.nix {inherit fleetixGpu;})
          (import ../modules/home/desktop/firefox-gpu.nix {inherit wrapper-manager;})
          (import ../modules/home/desktop/chromium-gpu.nix {inherit wrapper-manager;})
          ../modules/home/desktop/mpv-gpu.nix
          ({lib, ...}: {
            options.assertions = lib.mkOption {
              type = lib.types.listOf lib.types.attrs;
              default = [];
            };
            options.programs.modde.gpu.renderNode = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
            };
            options.programs.mpv.config = lib.mkOption {
              type = lib.types.attrsOf lib.types.str;
              default = {};
            };
          })
        ]
        ++ extraModules;
    }).config;
  standalone = evaluate {gpuRoutes = routes;} [];
  integrated = evaluate {gpuRoutes = routes;} [
    {
      _module.args.osConfig.canix-toolbelt = {
        gpuRender = {
          enable = true;
          renderNode = overrideNode;
        };
        gpuMedia = routes.media // {renderNode = overrideNode;};
      };
    }
  ];
  overridden =
    evaluate {
      gpuRoutes = routes;
      gpuMedia = routes.media;
    } [
      {
        canix-toolbelt.gpuMedia.renderNode = overrideNode;
        programs.modde.gpu.renderNode = overrideNode;
        programs.mpv.config.hwdec = "no";
      }
    ];
  disabled = evaluate {gpuRoutes = routes;} [{canix-toolbelt.gpuMedia.enable = false;}];
  empty = evaluate {} [];
  package = {
    name = "probe";
    pname = "probe";
    version = "1";
  };
  firefox = cfg: cfg.canix-toolbelt.firefoxGpu.wrap {basePackage = package;};
  chromium = cfg: cfg.canix-toolbelt.chromiumGpu.wrap {basePackage = package;};
in
  assert standalone.programs.modde.gpu.renderNode == renderNode;
  assert standalone.programs.mpv.config
  == {
    hwdec = "vaapi-copy";
    vaapi-device = mediaNode;
  };
  assert (firefox standalone).testEnv.MOZ_DRM_DEVICE.value == mediaNode;
  assert builtins.elem "--render-node-override=${mediaNode}" (chromium standalone).testFlags;
  assert integrated.programs.modde.gpu.renderNode == overrideNode;
  assert integrated.programs.mpv.config.vaapi-device == overrideNode;
  assert (firefox integrated).testEnv.MOZ_DRM_DEVICE.value == overrideNode;
  assert overridden.programs.modde.gpu.renderNode == overrideNode;
  assert overridden.programs.mpv.config.hwdec == "no" && overridden.programs.mpv.config.vaapi-device == overrideNode;
  assert (firefox overridden).testEnv.MOZ_DRM_DEVICE.value == overrideNode;
  assert builtins.elem "--render-node-override=${overrideNode}" (chromium overridden).testFlags;
  assert (firefox disabled).testEnv == {};
  assert disabled.programs.mpv.config == {hwdec = "auto-safe";};
  assert (import ../lib/mpvGpu.nix) (routes.media // {vendor = "nvidia";}) == {hwdec = "nvdec-copy";};
  assert !(builtins.elem "--render-node-override=${mediaNode}" (chromium disabled).testFlags);
  assert empty.programs.modde.gpu.renderNode == null && (firefox empty).testEnv == {};
  assert !((chromium standalone).testEnv ? DRI_PRIME);
  assert lib.all (cfg: lib.all (a: a.assertion) cfg.assertions) [standalone integrated overridden disabled empty];
    pkgs.writeText "gpu-routes-eval" "ok"
