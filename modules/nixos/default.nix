{
  activation-contracts = ./activation-contracts.nix;
  activation-manifest = ./activation-manifest.nix;

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
  disko-bcache-initrd = ./hardware/disko-bcache-initrd.nix;

  amd-sensors-boot = ./hardware/amd-sensors-boot.nix;
  boot-assessment = ./hardware/boot-assessment.nix;
  fan-control-thinkfan = ./hardware/fan-control-thinkfan.nix;
  goxlr = ./hardware/goxlr.nix;
  openrgb = ./hardware/openrgb.nix;
  razer = ./hardware/razer.nix;
  usb-tty = ./hardware/usb-tty.nix;
  vial = ./hardware/vial.nix;
  watchdog = ./hardware/watchdog.nix;
  wooting = ./hardware/wooting.nix;

  # Services
  betterdesk-server = ./services/betterdesk-server.nix;
  attic-projects-registry = ./services/attic-projects-registry.nix;
  atticd-preset = ./services/atticd-preset.nix;
  caddy-base = ./services/caddy-base.nix;
  caddy-service-registry = ./services/caddy-service-registry.nix;
  dev-agent-isolation = ./services/dev-agent-isolation.nix;
  dev-oom-guard = ./services/dev-oom-guard.nix;
  dns-octodns-cloudflare = ./services/dns-octodns-cloudflare.nix;
  garage = ./services/garage.nix;
  host-registry = ./registry/hosts.nix;
  impure-files = ./registry/impure-files.nix;
  initrd-ssh = ./services/initrd-ssh.nix;
  kanidm-preset = ./services/kanidm-preset.nix;
  power-cycle-relay = ./services/power-cycle-relay.nix;
  pypi-server = ./services/pypi-server.nix;
  rauthy-preset = ./services/rauthy-preset.nix;
  rbac = ./registry/rbac.nix;
  samba = ./services/samba.nix;
  service-registry = ./registry/services.nix;
  wol-relay = ./services/wol-relay.nix;

  forgejo-runner = ./services/forgejo-runner.nix;
  forgejo-runner-container-runtime = ./services/forgejo-runner-container-runtime.nix;
  pg-backup = ./services/pg-backup.nix;
  direct-link = ./networking/direct-link.nix;
  networkmanager-defaults = ./networking/networkmanager-defaults.nix;
  stalwart-seed-accounts = ./services/stalwart-seed-accounts.nix;
  sunshine = ./services/sunshine.nix;
  vpn-dns = ./networking/vpn-dns.nix;
  wg-home-client = ./networking/wg-home-client.nix;
  wg-home-shared = ./networking/wg-home-shared.nix;

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

  # CLI Tools
  cli-tools = ./cli-tools.nix;
}
