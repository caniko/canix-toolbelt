# Batched nix-direnv input roots

Import `canix-toolbelt.homeModules.direnv` and enable
`canix-toolbelt.direnv.enable`. The module enables direnv and nix-direnv and
patches the selected `canix-toolbelt.direnv.package`, preserving existing patches.

Source inputs are deduplicated and rooted in one Nix store operation with source
compilation disabled. Archive/root failures propagate even when the caller uses
`use flake ... || return 1`; a fresh environment cache is published only after
its input roots succeed. The package hook patches the installed script after
resholve, whose installation otherwise copies the original store input over
unpacked-tree changes. Existing base-package patches and fixup hooks are retained.
The `direnv-eval` check exercises the complete packaged `use_flake` function with
duplicate and empty input sets and verifies that archive/root failures retain
the previous cache, including conditional calls. Old cache/root cleanup occurs
after successful renewal, and the real-root VM checks Nix's indirect root links
for both unique inputs with source compilation disabled. The unpatched installed
script must fail the same renewal regression harness in hosted CI.

Consumers own shell integration, approval policy, checkout/source watches and
command-scoped GC reserves. This module does not authorize checkout hooks.
