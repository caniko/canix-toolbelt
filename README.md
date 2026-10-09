# canix-toolbelt

<!-- simit:badges:start -->

[![CI](https://img.shields.io/badge/CI-managed+extra-2088ff)](.github/workflows/ci.yaml) [![docs](https://img.shields.io/badge/docs-enabled-6f42c1)](https://docs.rs/canix-toolbelt) [![crates.io](https://img.shields.io/badge/crates.io-ready-f46623)](https://crates.io/crates/canix-toolbelt)

<!-- simit:badges:end -->

The Rust library and standalone CLI candidate are documented in [RUST.md](RUST.md).
They provide Cargo-native runtime-manifest loading and revision-bound PR review
and merge operations; publication steps and verification evidence are tracked
in [RELEASE.md](RELEASE.md).

Reusable, host-agnostic Nix building blocks extracted from
[caniko/canix](https://github.com/caniko/canix). The pieces here are the
hyper-stable parts of that repo — modules and helpers that have low churn,
no host/secret coupling, and are useful to other NixOS users.

Two output families:

- **`nixosModules.*`** — small, composable NixOS modules (CPU microcode, GPU
  vendor stacks, EFI/btrfs/pipewire/fwupd defaults, boot-assessment, hardware
  watchdog).
- **`flakeModules.*`** — `flake-parts` modules for `treefmt-nix`, git-hooks,
  host-output selection, and a generic ripgrep-based structure/boundary check.
- **`lib.mk*Site` helpers** — reusable website plumbing for tools that publish
  Zola sites, mdBook docs, combined static trees, and Codeberg Pages deploy
  apps without carrying per-project copies.

## Usage

### Live executor admission

`nixosModules.executor-admission` binds independently enabled workers to live
toolbelt profiles and registers their admission files as activation artifacts.
Detaching a profile closes new admission without changing worker units or their
restart triggers. See [executor admission](docs/executor-admission.md).

### Hardware-key pinentry

`homeModules.pinentry` selects an origin-relative Zellij popup, Qt, or the
requesting terminal for GPG and rage/age-plugin PIN requests. Packaged callers
use `lib.pinentry.mkPackages` or `lib.pinentry.mkRage`; see
[hardware-key pinentry](docs/pinentry.md) for integration and routing details.

### OpenCode Claude subscriptions

`homeModules.opencode-claude` wires the native V2 plugin through Jev to a
per-user Meridian service, using Fleetix-declared loopback endpoints. See
[Claude subscription integration](docs/opencode-claude.md) for configuration,
login and lifecycle details.

### GPU routing

Fleetix owns the typed Pkl GPU contract and Nix projections. Supply
`gpuRoutes = fleetix.lib.gpu.routes host.gpu` as a special argument, or use
the legacy `gpuMedia`/`gpuRender` arguments. Integrated Home Manager inherits
NixOS routes; standalone Home Manager uses topology defaults. Explicit Home
Manager route settings override either.

Import `homeModules.modde-gpu` alongside modde's Home Manager module. The
adapter supplies `programs.modde.gpu.renderNode` as a default. Explicit
application settings and saved installation choices take precedence. An
enabled route requires a stable `/dev/dri/by-path/pci-...-render` alias and a
modde module that supports GPU routing. modde validates the live device at
launch; proprietary NVIDIA routing uses explicit launch environment settings.

Rendering, media and compute remain independent roles. Browser wrappers read
the resolved `canix-toolbelt.gpuMedia` route. Import `homeModules.gpu-media`
alongside media adapters. `homeModules.mpv-gpu` supplies default
`hwdec`/`vaapi-device` settings with copy-mode decoding, preserves automatic
presentation selection, and uses `nvdec-copy` for NVIDIA without a VA-API
device. Explicit mpv settings override those defaults. Missing routes stay
disabled; no adapter globally sets `DRI_PRIME`.

### Module composition

```nix
{
  inputs.canix-toolbelt.url = "git+ssh://git@github.com/caniko/canix-toolbelt.git";

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

### Cloud-host configurator

`nixosModules.cloud-host` composes a complete small NixOS guest using Disko and
NetworkManager. Enable it explicitly and supply the disk, firmware, network
selector, state version and operator public keys:

```nix
{
  imports = [inputs.canix-toolbelt.nixosModules.cloud-host];
  nixpkgs.hostPlatform = "x86_64-linux";
  networking.hostName = "edge";
  canix-toolbelt.cloudHost = {
    enable = true;
    stateVersion = "25.11"; # keep the host's initial value on upgrades
    platform = "qemu";
    boot.mode = "uefi";
    disk = "/dev/disk/by-id/virtio-root"; # verify against this instance
    network.interfaceName = "ens3"; # or network.macAddress
    access = {
      port = 1337;
      authorizedKeys = [ (builtins.readFile ./operator.pub) ];
    };
  };
}
```

Use the existing `nixpkgs.lib.nixosSystem` or your fleet's configuration builder;
the module does not introduce another host registry. The normal NixOS options
remain the interface for extra accounts, packages, services, and secret managers.
The exported module imports its pinned Disko module. When composing with a fleet
that already imports Disko, make `inputs.canix-toolbelt.inputs.disko.follows = "disko"`
so both use the same module identity.

Firmware is required: `uefi` selects systemd-boot with a 512 MiB FAT ESP;
`bios` selects GRUB with a GPT BIOS boot partition. BIOS is x86_64 only; UEFI
supports x86_64 and aarch64. Both use Btrfs with `compress=zstd:3` and `noatime`:

| Subvolume | Mount | Recovery boundary |
| --- | --- | --- |
| `@root` | `/` | Snapshotted operating-system files |
| `@nix` | `/nix` | Store, profiles and GC roots retained independently |
| `@state` | `/var/lib` | Service databases, WireGuard identity and ACME state |
| `@log` | `/var/log` | Logs retained across root recovery |
| `@identity` | `/etc/ssh` | SSH host identity retained across root recovery |
| `@snapshots` | `/.snapshots` | Root-only Snapper snapshots, root-readable |

Weekly scrubbing covers the shared filesystem once, limited to 32 MiB/s. Daily
root snapshots retain seven daily entries; numbered cleanup keeps five ordinary
and five important snapshots, subject to Snapper's minimum age. Configure
`storage.snapshots.*` and `storage.compressionLevel` as needed. These are count
limits, not byte quotas or backups. Service state requires application-aware
backup; keep persistent WireGuard keys under `/var/lib`, or regenerate runtime
secrets with the fleet secret manager.

Use NixOS generation rollback for deployments. For filesystem recovery, retain
the corresponding Nix closure with a GC root before the checkpoint, recover the
required root files, and activate the matching generation. Disko mounts `@root`
explicitly: `snapper rollback` changing the filesystem's default subvolume is not
a boot-selection mechanism here. Whole-root replacement requires an offline
recovery procedure and a writable snapshot restored as `@root`; identities and
service state stay on their separate subvolumes. Disko partitioning is an explicit
installation operation, never an activation action. Reinstalling onto the same
disk is destructive.

The profile keeps `/tmp`, `/var/tmp`, `/home`, `/srv` and `/var` as ordinary
directories unless explicitly mounted separately. This overrides systemd's
implicit nested-subvolume creation: Btrfs root snapshots otherwise leave
unwritable placeholders for those subvolumes, preventing services with
`PrivateTmp` from starting after recovery. Add further subvolumes only with an
explicit mount and recovery policy.

`platform = "qemu"` supplies virtio drivers and the QEMU guest agent. `custom`
leaves additional drivers to the platform module. Neither setting selects a cloud
vendor. Provider selection must still verify actual firmware, disk/NIC identity,
serial-console/recovery behavior and addressing requirements.

IPv4 defaults to DHCP; IPv6 defaults to disabled. Static/routed providers can use:

```nix
canix-toolbelt.cloudHost.network.ipv4 = {
  method = "manual";
  addresses = [ "192.0.2.10/32" ];
  gateway = "192.0.2.1";
  gatewayOnLink = true;
  dns = [ "192.0.2.53" ];
};
```

IPv6 uses the same fields, with IPv6 literals. Manual addressing requires
addresses, gateway and DNS. A `/32` or `/128` routed uplink can mark its gateway
on-link explicitly. NetworkManager is the sole runtime manager and does not
create automatic competing DHCP profiles. Test provider reachability before
enrollment; syntactically valid addresses are not proof of working routing.

SSH is key-only, including root administration before any VPN is established.
The module opens only its administrative port; service roles add their own
listeners. When forwarding public Git SSH on port 22, choose a different
administrative port. Bootstrap host-key verification, installed-key enrollment,
secret rekeying and cloud-firewall restrictions belong to the installer/fleet
integration. This module does not perform those operations.

Small-host defaults are zero local build jobs, 256/1024 MiB Nix GC watermarks,
128 MiB persistent journal budget, five boot-menu generations, and weekly
age-based GC after 30 days. `resources.*` configures those sizes and local build
jobs. Boot-menu limits do not protect generations from GC; retain any required
rollback closure through the deployment system's GC roots. Supply the build-host
and private-cache policy from the fleet rather than putting credentials here.

Checks:

- `cloud-host-eval` forces full UEFI, BIOS, ARM UEFI, MAC-matched and routed
  static NixOS system derivations, and checks disabled behavior and invalid input.
- `cloud-host-install-bios` / `cloud-host-install-uefi` use Disko's installation
  test to partition empty virtual disks, boot, repeat activation and reboot while
  checking Btrfs mounts, retained host keys/service state, root-file recovery,
  numbered snapshot cleanup and NixOS specialisation rollback with a persistent
  GC root. They also restore the whole root offline, then check writable
  directories, service startup, activation and retained identity/state. The
  runner's shared store is read-only; a separate writable store on `@nix` holds
  the full system closure while an unreferenced path is collected and retained
  contents are verified. Provider operation, tunnel access and key enrollment
  have separate gates.

These checks must pass before consumption. Canix operators evaluate through
`canix repo eval` and opt into VM realization with
`canix cache binary build .#checks.x86_64-linux.cloud-host-install-uefi --include-tests --no-push`.
Use the analogous BIOS check. Provider-backed deployment is a separate gate.

### Public edge role

`nixosModules.public-edge` adds an optional gateway role to a cloud host or any
other NixOS machine. `nixosModules.edge-transport` is also available separately
for home peers. It uses a dedicated kernel WireGuard interface with exact peer
`/32` routes; public client prefixes and default routes are never installed.

```nix
{
  imports = [inputs.canix-toolbelt.nixosModules.public-edge];
  # Administrative SSH must not collide with forwarded Git SSH on port 22.
  canix-toolbelt.cloudHost.access.port = 1337;
  canix-toolbelt.networking.edgeTransport = {
    enable = true;
    address = "10.77.0.1";
    privateKeyFile = "/run/agenix/wg-edge";
    listenPort = 51821;
    mtu = 1380; # Select from measurements of the actual underlay.
    openFirewall = true;
    peers.home = {
      address = "10.77.0.2";
      publicKey = "<home-peer-public-key>";
    };
  };
  canix-toolbelt.services.publicEdge = {
    enable = true;
    publicAddress = "192.0.2.10";
    publicInterface = "ens3";
    http."app.example.org" = {
      serverName = "origin.example.org";
      upstreams = [{address = "10.77.0.2"; port = 443;}];
    };
    tcp.git = {publicPort = 22; address = "10.77.0.2"; port = 22;};
    udp.webtransport = {publicPort = 8443; address = "10.77.0.2"; port = 8443;};
  };
}
```

The addresses above are documentation examples. Home imports `edge-transport`,
uses its own runtime private key, and sets the edge peer's `endpoint` to the
stable public IP/port with `keepaliveSeconds = 25`. Open only the required
origin ports on `wg-edge`. Endpoint DNS must not depend on the service being
forwarded. Supply certificates through `services.caddy.certificates` or issuer
policies through `services.caddy.tlsPolicies` under `canix-toolbelt`; provision
zone-scoped DNS-01 credentials at runtime when pre-cutover issuance needs them.

Public HTTPS offers HTTP/1.1, HTTP/2 and HTTP/3 with TLS early data disabled.
`http.<hostname>.originProtocols` independently selects verified upstream HTTPS
versions (default HTTP/1.1 + HTTP/2; HTTP/3 requires `["3"]`). Upstream addresses
must be enrolled tunnel peers. Host is preserved independently of TLS SNI. Active
health checks require a `200` response on `healthPath` (default `/`). Configure a
suitable unauthenticated health path for sites whose main page redirects or
requires login.

The edge replaces forwarding identity. Each home ingress/relay must separately
restrict source admission with `caddy.servers.<name>.cidrAllowlist` and specify
`trustedProxies` for strict right-to-left `X-Forwarded-For` parsing. Include known
earlier proxy hops when the relay receives a forwarded chain; never trust every
address merely because it is on a private subnet.

TCP uses HAProxy without TLS termination. Set `proxyProtocol = "v1"` only on an
origin listener configured to accept PROXY v1 from the edge's exact tunnel IP.
This supplies connection metadata, not application authentication. UDP forwarding
uses connection-scoped DNAT/SNAT and exact public-interface/address/port matches;
the origin sees the edge tunnel IP. Application QUIC/WebTransport stays with the
application on its own UDP port. Caddy's HTTP/3 listener does not substitute for
WebTransport support in an application.

`checks.<system>.public-edge-eval` checks composition and rejects conflicting
ports, unenrolled destinations and invalid protocol combinations, then validates
the rendered Caddy and HAProxy configurations. The x86_64 `public-edge` VM test
exercises the packet path, including header spoofing, protocol fallback, large
transfers and the application-owned WebTransport fixture. It does not certify a
cloud provider, Stalwart, a browser's WebTransport implementation, or paid transit
capacity. See Canix's public-edge design for the enrollment and publication gates.

### GPU compute requests

`lib.gpu.normalize` and `lib.gpu.forHost` expose an optional `compute` record:
`{ dgpu = "intel"; compute.backend = "oneapi"; }` normalizes to a primary Intel
GPU with `compute.backend = "oneapi"`. Requests are validated against the
primary GPU (dGPU, otherwise iGPU): Intel/oneAPI, AMD/ROCm, NVIDIA/CUDA.
Absent or null declarations normalize to `compute = null` and do not opt into
acceleration. Consumers own package sources and runtime device selection.

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
    canix-toolbelt.url = "git+ssh://git@github.com/caniko/canix-toolbelt.git";
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

### Host-output selection

`flakeModules.host-selection` provides one flake-level policy for projects
that derive several outputs from the same host registry. Hosts omitted from
the policy stay enabled; setting a host to `enable = false` lets consumers
filter their NixOS, Home Manager, and installer outputs while retaining the
host in inventory data such as topology and SSH records.

```nix
{
  imports = [canix-toolbelt.flakeModules.host-selection];

  canix-toolbelt.host-selection.bar.enable = false;
}
```

### Site helpers

The site helpers build plain static directories that can be reused by any
static host. Pages deployment is owned by Plinth's `mkDeployPagesApp`.

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
