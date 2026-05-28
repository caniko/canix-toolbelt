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
- **`lib.mk*Site` helpers** — reusable website plumbing for tools that publish
  Zola sites, mdBook docs, combined static trees, and Codeberg Pages deploy
  apps without carrying per-project copies.

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
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
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
        canix-toolbelt.pre-commit.mypy = {
          enable = true;
          excludes = ["^vendor/" "^submodules/"];
        };

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

### Site helpers

The site helpers target the Codeberg Pages workflow used by caniko tools, but
the build outputs are plain static directories and can be reused by any static
host. The deploy helper reads the git remote name from an environment variable
(`DEPLOY_REMOTE` by default), so CI owns token injection and remote URLs.

`lib.mkZolaSite` builds a Zola source tree. `dataFiles` copies generated or
external files into the site tree before `zola build`; `theme = { name, src; }`
injects an AdiDoks-style theme under `themes/<name>`.

```nix
packages.${system}.website = canix-toolbelt.lib.mkZolaSite {
  inherit pkgs;
  src = ./website;
  dataFiles."data/capability-matrix.toml" = ./docs/capability-matrix.toml;
  theme = {
    name = "adidoks";
    src = inputs.adidoks;
  };
};
```

`lib.mkMdBookDocs` builds an mdBook source tree as-is. Keep themes and book
settings in `book.toml`; the helper does not impose an mdBook theme.

```nix
packages.${system}.docs = canix-toolbelt.lib.mkMdBookDocs {
  inherit pkgs;
  src = ./docs;
};
```

`lib.mkCombinedSite` copies a website to the root and docs under `/docs/`.
Pass `domains = null` to omit `.domains`, or a non-empty list to emit it for
Codeberg Pages. Codeberg treats the first line as canonical, so order matters.

```nix
packages.${system}.site = canix-toolbelt.lib.mkCombinedSite {
  inherit pkgs;
  website = config.packages.website;
  docs = config.packages.docs;
  domains = ["example.org" "www.example.org"];
};
```

`lib.mkDeployPagesApp` returns a `writeShellApplication` app that force-updates
the Pages branch from a built static site package. It never embeds a token or
remote URL; CI should create a git remote and set `DEPLOY_REMOTE` to that remote
name before running it.

```nix
apps.${system}.deploy-pages = {
  type = "app";
  program = "${
    canix-toolbelt.lib.mkDeployPagesApp {
      inherit pkgs;
      sitePackage = config.packages.site;
    }
  }/bin/deploy-pages";
};
```

Consumers using `flake-parts` can import `flakeModules.pages-deploy`, a thin
wrapper over `mkDeployPagesApp` that registers `apps.deploy-pages`.

```nix
{
  imports = [inputs.canix-toolbelt.flakeModules.pages-deploy];

  perSystem = {config, ...}: {
    canix-toolbelt.pages-deploy = {
      enable = true;
      sitePackage = config.packages.site;
      branch = "pages";
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
