{pkgs}: let
  inherit
    ((pkgs.lib.evalModules {
      specialArgs = {inherit pkgs;};
      modules = [
        ../modules/home/direnv.nix
        {
          options.programs.direnv = pkgs.lib.mkOption {type = pkgs.lib.types.attrs;};
          config.canix-toolbelt.direnv.enable = true;
        }
      ];
    }))
    config
    ;
  package = config.programs.direnv.nix-direnv.package;
in
  pkgs.testers.runNixOSTest {
    name = "toolbelt-direnv-real-roots";
    nodes.machine = {
      environment.systemPackages = [pkgs.bash pkgs.nix pkgs.coreutils];
      nix.settings.experimental-features = ["nix-command" "flakes"];
    };
    testScript = ''
      start_all()
      machine.wait_for_unit("multi-user.target")
      machine.succeed("mkdir -p /run/direnv-fixture")
      machine.succeed("cd /run/direnv-fixture; REAL_ROOTS=1 NIX_ROOT_BINARY=${pkgs.nix}/bin/nix NIX_STORE_BINARY=${pkgs.nix}/bin/nix-store bash ${./test-direnv-roots.sh} ${package}/share/nix-direnv/direnvrc")
    '';
  }
