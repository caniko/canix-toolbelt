# Batched nix-direnv input roots

Import `canix-toolbelt.homeModules.direnv` and enable
`canix-toolbelt.direnv.enable`. The module enables direnv and nix-direnv and
patches the selected `canix-toolbelt.direnv.package`, preserving existing patches.

Source inputs are deduplicated and rooted in one Nix store operation with source
compilation disabled. Archive/root failures propagate even when the caller uses
`use flake ... || return 1`; a fresh environment cache is published only after
its input roots succeed. The `direnv-eval` check exercises the packaged archival
block with duplicate and empty input sets and verifies that root failure retains
the previous cache.

Consumers own shell integration, approval policy, checkout/source watches and
command-scoped GC reserves. This module does not authorize checkout hooks.
