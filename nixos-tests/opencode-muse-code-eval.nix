{pkgs}: let
  inherit (pkgs) lib;
  evaluate = settings:
    (lib.evalModules {
      modules = [
        ../modules/home/opencode-muse-code.nix
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
    }).config;
  disabled = evaluate {};
  v1 = evaluate {
    programs.opencode.enable = true;
    canix-toolbelt.opencodeMuseCode = {
      enable = true;
      apiVersion = "v1";
    };
  };
  v2 = evaluate {
    programs.opencode.enable = true;
    canix-toolbelt.opencodeMuseCode.enable = true;
  };
  imported = evaluate {
    programs.opencode.enable = true;
    canix-toolbelt.opencodeMuseCode = {
      enable = true;
      legacyAuthFile = "/private/auth.json";
    };
  };
  invalid = evaluate {
    canix-toolbelt.opencodeMuseCode = {
      enable = true;
      legacyAuthFile = "relative/auth.json";
    };
  };
in
  (import ./lib/eval-checks.nix {inherit pkgs;}).mkEvalCheck {
    name = "opencode-muse-code-eval";
    assertions = [
      {
        name = "disabled-is-inert";
        assertion = disabled.programs.opencode.settings == {} && disabled.assertions == [];
        message = "Importing the adapter must not enable a provider or credential import.";
      }
      {
        name = "v1-only-legacy-entrypoint";
        assertion = builtins.length v1.programs.opencode.settings.plugin == 1 && !(v1.programs.opencode.settings ? plugins);
        message = "V1 must receive only its own plugin API.";
      }
      {
        name = "v2-default-no-credential-import";
        assertion = (builtins.head v2.programs.opencode.settings.plugins).options == {} && !(v2.programs.opencode.settings ? plugin);
        message = "V2 must use its native API with no implicit credential import.";
      }
      {
        name = "explicit-runtime-import";
        assertion = (builtins.head imported.programs.opencode.settings.plugins).options.legacyAuthFile == "/private/auth.json";
        message = "Credential import must retain the supplied runtime path.";
      }
      {
        name = "valid-consumers";
        assertion = lib.all (entry: entry.assertion) (v1.assertions ++ v2.assertions ++ imported.assertions);
        message = "Enabled adapters must accept configured OpenCode consumers.";
      }
      {
        name = "invalid-consumer-rejected";
        assertion = lib.all (entry: !entry.assertion) invalid.assertions;
        message = "Disabled OpenCode and relative credential paths must fail assertions.";
      }
    ];
  }
