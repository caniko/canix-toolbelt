# Home-manager modules, exposed as canix-toolbelt.homeModules.<name>.
#
# Mirrors modules/nixos, but is a *function* of the flake inputs the modules
# need: chromium-gpu builds its wrappers with wrapper-manager, which is closed
# over here so consumers don't have to pass `inputs.wrapper-manager` themselves.
{wrapper-manager}: {
  # GPU — iGPU render offload (DRI_PRIME) + VA-API decode-device selection
  igpu = ./desktop/igpu.nix;
  # GPU — Chromium VA-API acceleration flags/env wrapper
  chromium-gpu = import ./desktop/chromium-gpu.nix {inherit wrapper-manager;};
}
