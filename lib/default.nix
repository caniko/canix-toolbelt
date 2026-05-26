{
  agenixPaths = import ./agenixPaths.nix;
  caddy = import ./caddy.nix;
  dns = import ./dns.nix;
  gpu = import ./gpu.nix;
  mkPkgs = import ./mkPkgs.nix;
  networkmanager = import ./networkmanager.nix;
  opsShellPackages = import ./opsShellPackages.nix;
  peerRoute = import ./peerRoute.nix;
  storage = import ./storage.nix;
}
