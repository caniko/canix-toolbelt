# Home-manager modules, exposed as canix-toolbelt.homeModules.<name>.
#
# Mirrors modules/nixos, but is a *function* of the flake inputs the modules
# need: chromium-gpu builds its wrappers with wrapper-manager, which is closed
# over here so consumers don't have to pass `inputs.wrapper-manager` themselves.
{
  wrapper-manager,
  fleetixGpu,
}: {
  # Shared readiness contracts for Home Manager features with activation
  # hooks, runtime credentials, or mutable external state.
  activation-contracts = ./activation-contracts.nix;

  # Shared typed prompt policy and OpenCode renderer. Consumers add
  # `programs.<name>.autoSafe` declarations and this module compiles them into
  # the backend permission object.
  agent-safety = ./agent-safety.nix;
  opencode-environment = ./opencode-environment.nix;
  project-tree = ./project-tree.nix;
  roborev = ./roborev;
  browser-connection = ./browser-connection.nix;
  direnv = ./direnv.nix;
  opencode-muse-code = ./opencode-muse-code.nix;

  # GPU — explicit VA-API media-decode route (replaced the retired igpu API)
  gpu-media = import ../gpu-media.nix {inherit fleetixGpu;};
  gpu-render = import ../gpu-render.nix {inherit fleetixGpu;};
  modde-gpu = import ./gaming/modde-gpu.nix {inherit fleetixGpu;};
  mpv-gpu = ./desktop/mpv-gpu.nix;
  # GPU — Chromium VA-API acceleration flags/env wrapper
  chromium-gpu = import ./desktop/chromium-gpu.nix {inherit wrapper-manager;};
  # GPU — Firefox/Floorp VA-API decode wrapper (MOZ_DRM_DEVICE + libva)
  firefox-gpu = import ./desktop/firefox-gpu.nix {inherit wrapper-manager;};
  # COSMIC — switch keyboard layouts when more than one is configured
  keyboard-layout-shortcut = ./desktop/keyboard-layout-shortcut.nix;
}
