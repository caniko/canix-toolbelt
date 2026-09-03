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
              kind = "group";
              authentication = {
                required = true;
                method = "glab";
              };
              includeSubgroups = true;
            }
          ];
          repositoryPolicies = [
            {
              repository = "demo";
              canonical = "gitlab.example.test/team/nested/demo";
              mirrors = ["codeberg.example.test/team/demo"];
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
          == "repos/owned/demo";
        message = "owned project paths must be flat under repos/owned";
      }
      {
        name = "owned-repository-is-a-leaf";
        assertion =
          !(builtins.tryEval (projectTree.projectPath {
            class = "owned";
            repository = "codeberg.org/caniko/demo";
          })).success;
        message = "forge and namespace must not become owned checkout path components";
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
          == "worktrees/demo/fix";
        message = "worktree paths must follow repository/purpose";
      }
      {
        name = "rendered-schema";
        assertion = rendered.schemaVersion == 2 && rendered.root == "/srv/projects";
        message = "the Home Manager module must render schema version and root";
      }
      {
        name = "rendered-source";
        assertion =
          (builtins.head rendered.sources).namespace
          == "team/nested"
          && (builtins.head rendered.sources).kind == "group"
          && (builtins.head rendered.sources).authentication.method == "glab";
        message = "multi-segment namespaces must survive rendering";
      }
      {
        name = "rendered-repository-policy";
        assertion =
          (builtins.head rendered.repositoryPolicies).canonical
          == "gitlab.example.test/team/nested/demo";
        message = "canonical repository and mirror policy must survive rendering";
      }
    ];
  }
