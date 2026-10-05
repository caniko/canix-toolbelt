{pkgs}: let
  inherit (pkgs) lib;
  base = pkgs.nix-direnv.overrideAttrs (old: {
    patches = (old.patches or []) ++ [existingPatch];
    postFixup = (old.postFixup or "") + ''touch "$out/existing-hook"'';
  });
  existingPatch = pkgs.writeText "existing-direnv.patch" ''
    --- a/share/nix-direnv/direnvrc
    +++ b/share/nix-direnv/direnvrc
    @@ -1,2 +1,3 @@
     # -*- mode: sh -*-
     # shellcheck shell=bash
    +# Existing consumer patch retained.
  '';
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
          config.canix-toolbelt.direnv.package = base;
        }
      ];
    }).config;
  enabled = evaluate true;
  disabled = evaluate false;
  patched = enabled.programs.direnv.nix-direnv.package;
in
  assert disabled.programs.direnv == {};
  assert enabled.programs.direnv.enable && enabled.programs.direnv.nix-direnv.enable;
  assert lib.elem existingPatch patched.patches;
    pkgs.runCommand "direnv-eval" {
      nativeBuildInputs = [pkgs.bash pkgs.nix pkgs.coreutils];
    } ''
      test -f ${patched}/existing-hook
      mkdir negative positive
      # The unpatched installed output must fail this same regression harness.
      if (cd negative; bash ${./test-direnv-roots.sh} ${pkgs.nix-direnv}/share/nix-direnv/direnvrc); then
        echo 'Faulty renewal unexpectedly passed' >&2
        exit 1
      fi
      echo 'RED: unpatched installed nix-direnv rejected'
      (cd positive; bash ${./test-direnv-roots.sh} ${patched}/share/nix-direnv/direnvrc)
      touch "$out"
    ''
