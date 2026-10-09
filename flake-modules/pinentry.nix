{inputs, ...}: {
  perSystem = {pkgs, ...}: let
    packages = (import ../lib/pinentry.nix {inherit (pkgs) lib;}).mkPackages {inherit pkgs;};
  in {
    packages.canix-toolbelt-pinentry = packages.router;
    packages.rage-pinentry = packages.rage;
    packages.canix-toolbelt-zellij = packages.zellij;
    checks.pinentry-home-eval = import ../nixos-tests/pinentry-home-eval.nix {inherit inputs pkgs;};
  };
}
