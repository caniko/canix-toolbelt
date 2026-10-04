{
  config,
  lib,
  ...
}: let
  settings = (import ../../../lib/mpvGpu.nix) config.canix-toolbelt.gpuMedia;
in {
  programs.mpv.config = builtins.mapAttrs (_: lib.mkDefault) settings;
}
