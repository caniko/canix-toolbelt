{pkgs}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  projectTree = import ../lib/projectTree.nix {inherit lib;};
  evaluated = lib.evalModules {
    specialArgs = {inherit pkgs;};
    modules = [
      {
        options = {
          assertions = lib.mkOption {
            type = lib.types.listOf lib.types.attrs;
            default = [];
          };
          home.packages = lib.mkOption {
            type = lib.types.listOf lib.types.package;
            default = [];
          };
          xdg.configFile = lib.mkOption {
            type = lib.types.attrsOf (lib.types.submodule {
              options.text = lib.mkOption {type = lib.types.lines;};
            });
            default = {};
          };
        };
      }
      ../modules/home/project-tree.nix
      {
        canix-toolbelt.projectTree = {
          enable = true;
          root = "/srv/projects";
          controlRemote = "ssh://git@example.test/team/projects.git";
          sources = [
            {
              provider = "gitlab";
              host = "gitlab.example.test";
              namespace = "team/nested";
              includeSubgroups = true;
            }
          ];
          exclusions = [
            {
              coordinate = "gitlab.example.test/team/nested/private";
              reason = "protected personal repository";
            }
          ];
        };
      }
    ];
  };
  rendered = builtins.fromJSON evaluated.config.xdg.configFile."canix/project-tree.json".text;
in
  mkEvalCheck {
    name = "project-tree-eval";
    resultMessage = "project tree contract evaluated";
    assertions = [
      {
        name = "owned-path";
        assertion =
          projectTree.projectPath {
            class = "owned";
            forge = "codeberg.org";
            namespace = "caniko";
            repository = "demo";
          }
          == "owned/codeberg.org/caniko/demo";
        message = "owned project paths must follow class/forge/namespace/repository";
      }
      {
        name = "worktree-path";
        assertion =
          projectTree.worktreePath {
            forge = "gitlab.com";
            namespace = "team/nested";
            repository = "demo";
            purpose = "fix";
          }
          == "worktrees/gitlab.com/team/nested/demo/fix";
        message = "worktree paths must retain the full base coordinate";
      }
      {
        name = "rendered-schema";
        assertion = rendered.schemaVersion == 1 && rendered.root == "/srv/projects";
        message = "the Home Manager module must render schema version and root";
      }
      {
        name = "rendered-source";
        assertion = (builtins.head rendered.sources).namespace == "team/nested";
        message = "multi-segment namespaces must survive rendering";
      }
    ];
  }
