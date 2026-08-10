# Home-manager modules, exposed as canix-toolbelt.homeModules.<name>.
#
# Mirrors modules/nixos, but is a *function* of the flake inputs the modules
# need: chromium-gpu builds its wrappers with wrapper-manager, which is closed
# over here so consumers don't have to pass `inputs.wrapper-manager` themselves.
{
  wrapper-manager,
  goose,
}: {
  # Shared readiness contracts for Home Manager features with activation
  # hooks, runtime credentials, or mutable external state.
  activation-contracts = ./activation-contracts.nix;

  # Shared typed prompt policy and OpenCode renderer. Consumers add
  # `programs.<name>.autoSafe` declarations and this module compiles them into
  # the backend permission object.
  agent-safety = ./agent-safety.nix;
  project-tree = ./project-tree.nix;

  # GPU — explicit VA-API media-decode route (replaced the retired igpu API)
  gpu-media = ../gpu-media.nix;
  # GPU — Chromium VA-API acceleration flags/env wrapper
  chromium-gpu = import ./desktop/chromium-gpu.nix {inherit wrapper-manager;};
  # GPU — Firefox/Floorp VA-API decode wrapper (MOZ_DRM_DEVICE + libva)
  firefox-gpu = import ./desktop/firefox-gpu.nix {inherit wrapper-manager;};
  # COSMIC — switch keyboard layouts when more than one is configured
  keyboard-layout-shortcut = ./desktop/keyboard-layout-shortcut.nix;
  # AI — goose: the upstream Home Manager module plus canix's opinionated
  # defaults (enhanced CLI build with shell completions + man pages).
  goose = import ./ai/goose/default.nix {gooseFlake = goose;};
}
