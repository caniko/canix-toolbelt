# canix-toolbelt

Reusable, host-agnostic Nix building blocks extracted from
[caniko/canix](https://github.com/caniko/canix). The pieces here are the
hyper-stable parts of that repo — modules and helpers that have low churn,
no host/secret coupling, and are useful to other NixOS users.

Two output families:

- **`nixosModules.*`** — small, composable NixOS modules (CPU microcode, GPU
  vendor stacks, EFI/btrfs/pipewire/fwupd defaults, boot-assessment, hardware
  watchdog).
- **`flakeModules.*`** — `flake-parts` modules for `treefmt-nix`, git-hooks,
  and a generic ripgrep-based structure/boundary check.

## Usage

```nix
{
  inputs.canix-toolbelt.url = "git+ssh://git@codeberg.org/caniko/canix-toolbelt.git";

  outputs = {nixpkgs, canix-toolbelt, ...}: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        canix-toolbelt.nixosModules.hardware-base
        canix-toolbelt.nixosModules.cpu-amd
        canix-toolbelt.nixosModules.efi
        canix-toolbelt.nixosModules.pipewire
        canix-toolbelt.nixosModules.gpu-amd
        # ...your own configuration
      ];
    };
  };
}
```

### Dev-stack flake-parts module

```nix
{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    canix-toolbelt.url = "git+ssh://git@codeberg.org/caniko/canix-toolbelt.git";
  };

  outputs = inputs @ {flake-parts, canix-toolbelt, ...}:
    flake-parts.lib.mkFlake {inherit inputs;} {
      systems = ["x86_64-linux"];
      imports = [
        canix-toolbelt.flakeModules.dev-stack    # formatters + git-hooks
        canix-toolbelt.flakeModules.structure-check
      ];

      perSystem = {pkgs, ...}: {
        canix-toolbelt.structure-check = {
          enable = true;
          src = ./.;
          rules = [
            {
              name = "no-home-imports-in-root";
              pattern = "\\.\\./.*home/";
              paths = ["root"];
              globs = ["*.nix"];
              message = "root/ must not import home/ paths";
            }
          ];
        };
      };
    };
}
```

## Module index

### Hardware basics

| Module | What it does |
| --- | --- |
| `hardware-base` | Enables `hardware.enableRedistributableFirmware` |
| `cpu-amd` / `cpu-intel` | Microcode updates gated on redistributable firmware |
| `efi` | systemd-boot with `configurationLimit = 10` |
| `fwupd` | Enables fwupd |
| `pipewire` | Pipewire (alsa/jack/pulse), disables PulseAudio |
| `btrfs-autoscrub` | Weekly btrfs scrub |
| `boot-assessment` | UKI boot-counting auto-rollback (option-gated) |
| `watchdog` | Hardware watchdog with chipset selector |

### GPU

Vendor backends:

- `gpu-amd`, `gpu-mesa`, `gpu-nvidia`
- `gpu-backend`, `gpu-backend-opengl-only`, `gpu-backend-vulkan`

Intel:

- `gpu-intel` — base (intel-gpu-tools)
- `gpu-intel-compute` — compute runtime + level-zero
- `gpu-intel-media` — media driver + iHD VAAPI
- `gpu-intel-vulkan` — common + media + vulkan backend
- `gpu-intel-xe` — generic xe driver force-probe support
- `gpu-intel-hd-615` — Kaby Lake HD 615 (no Vulkan)
- `gpu-intel-arc-a770`, `gpu-intel-arc-a770-i915`

GPU model-specific modules read `canix-toolbelt.hardware.gpu.profile`. Example
profile values: `"intel-arc-a770-xe"`, `"intel-arc-a770-i915"`.

Helpers:

- `gpu-switcheroo` — switcheroo-control service + custom-built package

## Stability

Everything in this repo is extracted from canix where it has been running for
months without churn. The module API is intended to stay stable; breaking
changes to option names will be called out in commit messages.

## License

MIT
