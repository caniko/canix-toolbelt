{pkgs, ...}: {
  services.switcherooControl = {
    enable = true;
    package = pkgs.callPackage ./package.nix {};
  };
}
