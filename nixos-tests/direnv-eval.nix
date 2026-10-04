{pkgs}: let
  inherit (pkgs) lib;
  evaluate = enabled:
    (lib.evalModules {
      specialArgs = {inherit pkgs;};
      modules = [
        ../modules/home/direnv.nix
        {
          options.programs.direnv = lib.mkOption {
            type = lib.types.attrs;
            default = {};
          };
          config.canix-toolbelt.direnv.enable = enabled;
        }
      ];
    }).config;
  enabled = evaluate true;
  disabled = evaluate false;
  patched = enabled.programs.direnv.nix-direnv.package;
in
  assert disabled.programs.direnv == {};
  assert enabled.programs.direnv.enable && enabled.programs.direnv.nix-direnv.enable;
    pkgs.runCommand "direnv-eval" {
      nativeBuildInputs = [pkgs.bash pkgs.nix pkgs.coreutils];
    } ''
      bash ${./test-direnv-roots.sh} ${patched}/share/nix-direnv/direnvrc
      touch "$out"
    ''
