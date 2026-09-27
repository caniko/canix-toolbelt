{...}:
  # Use the project's existing flake-parts policy; only flakeCheck belongs to
  # the flake-parts integration rather than a standalone treefmt module.
  builtins.removeAttrs
  ((import ../flake-modules/formatters.nix {
      inputs.treefmt-nix.flakeModule = {};
    }).perSystem.treefmt)
  ["flakeCheck"]
