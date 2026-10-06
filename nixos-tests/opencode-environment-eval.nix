{pkgs}: let
  inherit (pkgs) lib;
  evaluate = settings:
    (lib.evalModules {
      modules = [
        ../modules/home/opencode-environment.nix
        {
          options = {
            assertions = lib.mkOption {
              type = lib.types.listOf lib.types.attrs;
              default = [];
            };
            programs.opencode = {
              enable = lib.mkEnableOption "OpenCode";
              settings = lib.mkOption {
                type = lib.types.attrs;
                default = {};
              };
            };
          };
        }
        settings
      ];
      specialArgs = {inherit pkgs;};
    }).config;
  disabled = evaluate {};
  enabled = evaluate {
    programs.opencode.enable = true;
    canix-toolbelt.opencodeEnvironment = {
      enable = true;
      package = pkgs.emptyDirectory;
      options = {
        roots = ["/workspaces/example"];
        direnvApproval = "manual";
      };
    };
  };
  invalid = evaluate {
    canix-toolbelt.opencodeEnvironment = {
      enable = true;
      package = pkgs.emptyDirectory;
    };
  };
in
  assert disabled.programs.opencode.settings == {} && disabled.assertions == [];
  assert lib.all (a: a.assertion) enabled.assertions;
  assert !(lib.all (a: a.assertion) invalid.assertions);
  assert builtins.length enabled.programs.opencode.settings.plugins == 1;
  assert (builtins.head enabled.programs.opencode.settings.plugins).options.roots == ["/workspaces/example"];
  assert (builtins.head enabled.programs.opencode.settings.plugins).options.direnvApproval == "manual";
    pkgs.writeText "opencode-environment-eval" "ok"
