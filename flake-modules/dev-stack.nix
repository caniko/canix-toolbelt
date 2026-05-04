# Bundled dev-stack: formatters + git-hooks. Import this single module to get
# both treefmt-nix and git-hooks.nix wired up with sensible defaults.
{
  imports = [
    ./formatters.nix
    ./git-hooks.nix
  ];
}
