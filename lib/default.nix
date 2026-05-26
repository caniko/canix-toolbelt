{lib}: {
  agenixPaths = import ./agenixPaths.nix;
  caddy = import ./caddy.nix {inherit lib;};
  dns = import ./dns.nix;
  facterDisks = import ./facterDisks.nix;
  gpu = import ./gpu.nix;
  mkPkgs = import ./mkPkgs.nix;
  networkmanager = import ./networkmanager.nix;
  opsShellPackages = import ./opsShellPackages.nix;
  peerRoute = import ./peerRoute.nix;
  rbac = import ./rbac.nix;
  sshAliases = import ./sshAliases.nix;
  storage = import ./storage.nix;
}
