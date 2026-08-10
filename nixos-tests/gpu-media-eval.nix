{
  inputs,
  pkgs,
}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  module = ../modules/gpu-media.nix;
  valid = {
    enable = true;
    vendor = "amd";
    renderNode = "/dev/dri/by-path/pci-0000:65:00.0-render";
    libvaDriver = "radeonsi";
  };
  evaluate = gpuMedia:
    lib.evalModules {
      specialArgs = {inherit gpuMedia;};
      modules = [
        (import "${pkgs.path}/nixos/modules/misc/assertions.nix")
        module
      ];
    };
  check = gpuMedia: let
    m = evaluate gpuMedia;
  in
    lib.tryEval (lib.asserts.checkAssertWarn m.config.assertions [] m.config);
  cfg = (evaluate valid).config.canix-toolbelt.gpuMedia;
  emptyCfg = (evaluate {}).config.canix-toolbelt.gpuMedia;
  incomplete = check {
    enable = true;
    vendor = "amd";
  };
  badNode = check (valid // {renderNode = "renderD128";});

  firewall = import ../lib/firefoxGpu.nix {
    lib = pkgs.lib;
    wrapper-manager = inputs.wrapper-manager;
  };
  fakeFf = pkgs.symlinkJoin {
    name = "fake-firefox";
    paths = [
      (pkgs.writeShellScriptBin "fake-firefox" ''
        printf '%s %s\n' "$MOZ_DRM_DEVICE" "$LIBVA_DRIVER_NAME"
      '')
    ];
  };
  ffMedia = firewall {
    inherit pkgs;
    basePackage = fakeFf;
    gpuMedia = valid;
  };
  ffPlain = firewall {
    inherit pkgs;
    basePackage = fakeFf;
    gpuMedia = {enable = false;};
  };
in
  mkEvalCheck {
    name = "gpu-media-eval";
    resultMessage = "gpuMedia module contract passed";
    assertions = [
      {
        name = "valid-route-surfaces";
        assertion = cfg.enable && cfg.vendor == "amd" && cfg.renderNode == valid.renderNode && cfg.libvaDriver == "radeonsi";
        message = "a configured route must surface on config.canix-toolbelt.gpuMedia";
      }
      {
        name = "defaults-disabled";
        assertion = !emptyCfg.enable && emptyCfg.vendor == null && emptyCfg.renderNode == null && emptyCfg.libvaDriver == null;
        message = "absent gpuMedia must default to a disabled route";
      }
      {
        name = "incomplete-route-rejected";
        assertion = !incomplete.success;
        message = "enable without renderNode/libvaDriver must fail assertions";
      }
      {
        name = "non-dri-node-rejected";
        assertion = !badNode.success;
        message = "renderNode outside /dev/dri/ must fail assertions";
      }
    ];
    runtimeScript = ''
      test "$(${ffMedia}/bin/fake-firefox)" = "/dev/dri/by-path/pci-0000:65:00.0-render radeonsi"
      test "$(${ffPlain}/bin/fake-firefox)" = " "
    '';
  }
