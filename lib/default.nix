{lib}: let
  site = import ./site.nix {inherit lib;};
in
  {
    agenixPaths = import ./agenixPaths.nix;
    agentSafety = import ./agent-safety.nix {inherit lib;};
    activation = import ./activation.nix {inherit lib;};
    caddy = import ./caddy.nix {inherit lib;};
    deviceTypes = import ./deviceTypes.nix;
    devAgentIsolation = import ./dev-agent-isolation.nix {inherit lib;};
    dns = import ./dns.nix;
    facterDisks = import ./facterDisks.nix;
    gpu = import ./gpu.nix;
    homeActivation = import ./homeActivation.nix {inherit lib;};
    hostSelection = import ./hostSelection.nix {inherit lib;};
    mkPkgs = import ./mkPkgs.nix;
    networkmanager = import ./networkmanager.nix {inherit lib;};
    nexus = import ./nexus.nix {inherit lib;};
    opsShellPackages = import ./opsShellPackages.nix;
    profiles = import ./profiles.nix {inherit lib;};
    peerRoute = import ./peerRoute.nix;
    projectTree = import ./projectTree.nix {inherit lib;};
    rbac = import ./rbac.nix;
    sshAliases = import ./sshAliases.nix;
    storage = import ./storage.nix;
    systemd = import ./systemd.nix {inherit lib;};
  }
  // site
