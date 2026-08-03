{
  inputs,
  pkgs,
}: let
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  inherit (inputs.nixpkgs) lib;

  eval = modules:
    lib.nixosSystem {
      inherit (pkgs.stdenv.hostPlatform) system;
      modules =
        modules
        ++ [
          {
            nixpkgs.config.allowUnsupportedSystem = true;
            nixpkgs.config.allowUnfree = true;
            system.stateVersion = "25.11";
          }
        ];
    };

  cases = [
    {
      evaluation = eval [../modules/nixos/hardware/gpu/backend/vulkan.nix];
      runtime = ["libvdpau" "vkbasalt" "vulkan-loader"];
    }
    {
      evaluation = eval [../modules/nixos/hardware/gpu/intel/with-vulkan.nix];
      runtime = ["intel-media-driver" "libvdpau-va-gl" "vkbasalt" "vpl-gpu-rt" "vulkan-loader"];
    }
    {
      evaluation = eval [
        ../modules/nixos/hardware/gpu/intel/arc/common.nix
        {canix-toolbelt.hardware.gpu.profile = "intel-arc-a770-xe";}
      ];
      runtime = ["intel-compute-runtime" "intel-media-driver" "level-zero" "libvdpau" "libvdpau-va-gl" "vkbasalt" "vpl-gpu-rt" "vulkan-loader"];
    }
    {
      evaluation = eval [../modules/nixos/hardware/gpu/intel/hd-615.nix];
      runtime = ["intel-media-driver" "libvdpau-va-gl" "vpl-gpu-rt" "vulkan-loader"];
    }
  ];

  diagnostics = ["libva-utils" "mangohud" "vulkan-tools" "vulkan-validation-layers"];
  package = case: name: case.evaluation.pkgs.${name};
  installedGpuPackages = case:
    case.evaluation.config.environment.systemPackages
    ++ case.evaluation.config.hardware.graphics.extraPackages;
in
  mkEvalCheck {
    name = "gpu-backends-eval";
    assertions = [
      {
        name = "diagnostics-absent";
        assertion = lib.all (case: lib.all (name: !(builtins.elem (package case name) (installedGpuPackages case))) diagnostics) cases;
        message = "generic GPU backends must not install diagnostic or development packages";
      }
      {
        name = "runtime-packages-remain";
        assertion = lib.all (case: lib.all (name: builtins.elem (package case name) case.evaluation.config.hardware.graphics.extraPackages) case.runtime) cases;
        message = "generic GPU backends must retain their runtime loaders, drivers, and layers";
      }
    ];
  }
