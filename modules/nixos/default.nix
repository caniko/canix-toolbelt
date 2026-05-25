{
  # Generic host config profiles toggled via specialisations
  profiles = ./profiles.nix;

  # Hardware
  hardware-base = ./hardware/base.nix;

  cpu-amd = ./hardware/cpu/amd.nix;
  cpu-intel = ./hardware/cpu/intel.nix;

  efi = ./hardware/efi.nix;
  fwupd = ./hardware/fwupd.nix;
  pipewire = ./hardware/pipewire.nix;
  btrfs-autoscrub = ./hardware/btrfs.nix;

  boot-assessment = ./hardware/boot-assessment.nix;
  watchdog = ./hardware/watchdog.nix;

  # Services
  forgejo-runner = ./services/forgejo-runner.nix;
  forgejo-runner-container-runtime = ./services/forgejo-runner-container-runtime.nix;
  stalwart-seed-accounts = ./services/stalwart-seed-accounts.nix;

  # GPU — backends
  gpu-backend = ./hardware/gpu/backend/common.nix;
  gpu-backend-opengl-only = ./hardware/gpu/backend/opengl-only.nix;
  gpu-backend-vulkan = ./hardware/gpu/backend/vulkan.nix;

  # GPU — vendors
  gpu-amd = ./hardware/gpu/amd.nix;
  gpu-mesa = ./hardware/gpu/mesa.nix;
  gpu-nvidia = ./hardware/gpu/nvidia.nix;

  # GPU — Intel
  gpu-intel = ./hardware/gpu/intel/common.nix;
  gpu-intel-compute = ./hardware/gpu/intel/compute.nix;
  gpu-intel-media = ./hardware/gpu/intel/media.nix;
  gpu-intel-vulkan = ./hardware/gpu/intel/with-vulkan.nix;
  gpu-intel-xe = ./hardware/gpu/intel/xe.nix;
  gpu-intel-hd-615 = ./hardware/gpu/intel/hd-615.nix;
  gpu-intel-arc-a770 = ./hardware/gpu/intel/arc/a770.nix;
  gpu-intel-arc-a770-i915 = ./hardware/gpu/intel/arc/a770-i915.nix;

  # GPU helpers
  gpu-switcheroo = ./hardware/gpu/switcheroo/switcheroo.nix;
}
