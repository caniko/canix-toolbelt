# Caddy helpers exposed under `legacyPackages`.
#
# `caddyPluginsVendorFor plugins hash` returns the vendor FOD inside
# `caddy.withPlugins`. Use it to discover the `pluginsHash` for a chosen
# plugin set without evaluating any NixOS host config or cross-building
# caddy itself: nixpkgs' `withPlugins` runs `xcaddy build && go mod vendor`
# inside a fixed-output derivation, so the hash is platform-independent —
# any cross or native build computes the same `got:` value.
#
# Curried so a CLI can call it as `caddyPluginsVendorFor plugins hash` and
# pass `hash = lib.fakeHash` to force a hash-mismatch error (revealing the
# real hash). A real hash makes the FOD succeed and produces a store path
# you can `attic push` to seed a cache.
#
# Usage:
#
#   imports = [inputs.canix-toolbelt.flakeModules.caddy-helpers];
#   # Then, from a shell:
#   #   nix build .#legacyPackages.x86_64-linux.caddyPluginsVendorFor \
#   #     --apply "f: f [ {repo = \"...\"; version = \"...\"; hash = \"...\";} ] \"sha256-AAA...=\""
{
  flake-parts-lib,
  lib,
  ...
}: {
  options.perSystem = flake-parts-lib.mkPerSystemOption ({
    config,
    pkgs,
    ...
  }: let
    cfg = config.canix-toolbelt.caddy-helpers;
  in {
    options.canix-toolbelt.caddy-helpers = {
      enable =
        lib.mkEnableOption "caddy plugin helpers under legacyPackages"
        // {default = true;};
    };

    config = lib.mkIf cfg.enable {
      legacyPackages.caddyPluginsVendorFor = plugins: hash:
        (pkgs.caddy.withPlugins {inherit plugins hash;}).src;
    };
  });
}
