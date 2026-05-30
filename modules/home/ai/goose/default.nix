# canix-toolbelt's goose Home Manager module.
#
# Wraps the upstream goose Home Manager module and layers canix's opinionated
# default: an "enhanced" CLI build with shell completions and man pages
# installed and the `disable-update` cargo feature enabled (a store-installed
# goose must not self-update). Everything else is the upstream module, so the
# full `programs.goose` option surface is available unchanged.
#
# ACP/agent provider preset package sets are exposed separately as
# `canix-toolbelt.lib.goose.acpPackages` (see ./presets.nix), so hosts opt in
# explicitly per provider rather than through magic defaults.
{gooseFlake}: {
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (pkgs.stdenv.hostPlatform) system;

  basePackage = gooseFlake.packages.${system}.default;

  enhancedCliPackage = basePackage.overrideAttrs (old: {
    nativeBuildInputs = (old.nativeBuildInputs or []) ++ [pkgs.installShellFiles];

    # The flake's `--package goose-cli` build already produces the
    # `generate_manpages` helper bin alongside `goose`; enable the
    # update-disabling feature for store installs.
    buildFeatures = (old.buildFeatures or []) ++ ["disable-update"];

    # The base package re-runs the goose-cli test suite; skip it for the
    # wrapped build (the base package already gates that).
    doCheck = false;

    postInstall =
      (old.postInstall or "")
      + ''
        completions_dir="$(mktemp -d)"
        trap 'rm -rf "$completions_dir"' EXIT

        $out/bin/goose completion bash > "$completions_dir/goose.bash"
        $out/bin/goose completion fish > "$completions_dir/goose.fish"
        $out/bin/goose completion zsh > "$completions_dir/goose.zsh"

        installShellCompletion --cmd goose \
          --bash "$completions_dir/goose.bash" \
          --fish "$completions_dir/goose.fish" \
          --zsh "$completions_dir/goose.zsh"

        if [ -x "$out/bin/generate_manpages" ]; then
          export CARGO_MANIFEST_DIR="$PWD/crates/goose-cli"
          "$out/bin/generate_manpages"
          installManPage target/man/*.1
          rm -f "$out/bin/generate_manpages"
        fi
      '';
  });
in {
  imports = [gooseFlake.homeManagerModules.goose];

  config = lib.mkIf config.programs.goose.cli.enable {
    programs.goose.cli.package = lib.mkDefault enhancedCliPackage;
  };
}
