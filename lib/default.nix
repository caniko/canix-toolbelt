{lib}: let
  site = import ./site.nix {inherit lib;};
in
  {
    agenixPaths = import ./agenixPaths.nix;
    activation = import ./activation.nix {inherit lib;};
    caddy = import ./caddy.nix {inherit lib;};
    deviceTypes = import ./deviceTypes.nix;
    dns = import ./dns.nix;
    facterDisks = import ./facterDisks.nix;
    gpu = import ./gpu.nix;
    homeActivation = import ./homeActivation.nix {inherit lib;};
    hostSelection = import ./hostSelection.nix {inherit lib;};
    mkPkgs = import ./mkPkgs.nix;
    networkmanager = import ./networkmanager.nix;
    nexus = import ./nexus.nix {inherit lib;};
    opencode = import ./opencode.nix {inherit lib;};
    opsShellPackages = import ./opsShellPackages.nix;
    peerRoute = import ./peerRoute.nix;
    rbac = import ./rbac.nix;
    sshAliases = import ./sshAliases.nix;
    storage = import ./storage.nix;
    systemd = import ./systemd.nix {inherit lib;};
  }
  // site
