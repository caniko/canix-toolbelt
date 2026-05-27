{lib}: {
  agenixPaths = import ./agenixPaths.nix;
  caddy = import ./caddy.nix {inherit lib;};
  deviceTypes = import ./deviceTypes.nix;
  dns = import ./dns.nix;
  facterDisks = import ./facterDisks.nix;
  gpu = import ./gpu.nix;
  mkPkgs = import ./mkPkgs.nix;
  networkmanager = import ./networkmanager.nix;
  nexus = import ./nexus.nix {inherit lib;};
  opsShellPackages = import ./opsShellPackages.nix;
  peerRoute = import ./peerRoute.nix;
  rbac = import ./rbac.nix;
  sshAliases = import ./sshAliases.nix;
  storage = import ./storage.nix;
}
