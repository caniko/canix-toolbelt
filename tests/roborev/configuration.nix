{
  pkgs,
  homeManager,
  package,
  moduleResults,
}: let
  schema = import ../../modules/home/roborev/schema.nix {
    inherit (pkgs) lib;
    toml = pkgs.formats.toml {};
  };
  profiles = ["claude" "codex" "copilot" "cursor" "droid" "gemini" "hermes" "qwen" "grok"];
  evaluate = extra:
    (homeManager.lib.homeManagerConfiguration {
      inherit pkgs;
      modules = [
        ../../modules/home/roborev
        {
          home.username = "fixture";
          home.homeDirectory = "/build/roborev-runtime-home";
          home.stateVersion = "26.05";
          programs.roborev = {
            enable = true;
            inherit package;
          };
        }
        extra
      ];
    }).config.programs.roborev;
  generated = evaluate {
    programs.roborev = {
      agentCommands.opencode = "${pkgs.coreutils}/bin/true";
      settings = {
        hook_timeout_seconds = 0;
        mouse_enabled = false;
        default_model = "";
        hidden_columns = [];
        projects = {};
      };
      repositories.fixture.settings = {
        review_md_fallback = false;
        max_prompt_size = 0;
        excluded_branches = [];
        ci.reviews = {};
      };
      integrations = pkgs.lib.genAttrs profiles (_: {
        hooks = {
          enable = true;
          settings.fixture_setting = "preserved";
        };
        mcp.enable = true;
        skills.enable = true;
      });
    };
  };
  private = evaluate {
    programs.roborev = {
      runtimeConfigFile = "/build/roborev private $literal %pct \"quote\" \\back.toml";
      environment.RR_FIXTURE_VALUE = "dollar $ percent % quote \"";
      colorMode = "dark";
    };
  };
  projection = pkgs.writeText "roborev-configuration-fixtures.json" (builtins.toJSON {
    inherit (schema) inventory;
    generatedConfig = generated._configFile;
    generatedBinary = "${generated.finalPackage}/bin/roborev";
    repositoryConfig = generated.repositories.fixture.configFile;
    integrations = pkgs.lib.mapAttrs (_: integration: toString integration.package) generated.integrations;
    runtimeConfig = private.runtimeConfigFile;
    configLink = private._configFile;
    binary = "${private.finalPackage}/bin/roborev";
    inherit (private) dataDir;
  });
  python = pkgs.python3.withPackages (p: [p.pyyaml]);
in
  assert pkgs.lib.assertMsg (builtins.all (value: value) (builtins.attrValues moduleResults)) "Roborev module evaluation regression";
    pkgs.runCommand "roborev-module-results" {nativeBuildInputs = [python];} ''
      mkdir -p "$out"
      cp ${pkgs.writeText "roborev-module-results.json" (builtins.toJSON moduleResults)} "$out/evaluation.json"
      ${python}/bin/python3 ${./.}/configuration.py ${projection} ${package.src} ${package.goModules} "$out"
    ''
