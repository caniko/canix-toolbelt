# Home-manager modules, exposed as canix-toolbelt.homeModules.<name>.
#
# Mirrors modules/nixos, but is a *function* of the flake inputs the modules
# need: chromium-gpu builds its wrappers with wrapper-manager, which is closed
# over here so consumers don't have to pass `inputs.wrapper-manager` themselves.
{
  wrapper-manager,
  goose,
}: {
  # GPU — iGPU render offload (DRI_PRIME) + VA-API decode-device selection
  igpu = ./desktop/igpu.nix;
  # GPU — Chromium VA-API acceleration flags/env wrapper
  chromium-gpu = import ./desktop/chromium-gpu.nix {inherit wrapper-manager;};
  # AI — goose: the upstream Home Manager module plus canix's opinionated
  # defaults (enhanced CLI build with shell completions + man pages).
  goose = import ./ai/goose/default.nix {gooseFlake = goose;};
}
