{pkgs}: let
  inherit (pkgs) lib;
  evaluate = enabled:
    lib.evalModules {
      specialArgs = {inherit pkgs;};
      modules = [
        ../modules/home/browser-connection.nix
        {
          options.home.packages = lib.mkOption {
            type = lib.types.listOf lib.types.package;
            default = [];
          };
        }
        {
          canix-toolbelt.browserConnection = {
            enable = enabled;
            headless = true;
          };
        }
      ];
    };
  enabled = (evaluate true).config;
  disabled = (evaluate false).config;
  adapter = enabled.canix-toolbelt.browserConnection.adapters.opencode;
in
  (import ./lib/eval-checks.nix {inherit pkgs;}).mkEvalCheck {
    name = "browser-connection-eval";
    assertions = [
      {
        name = "enabled-installs-one-adapter";
        assertion = builtins.length enabled.home.packages == 1;
        message = "enabled connection must install its adapter";
      }
      {
        name = "disabled-is-inert";
        assertion = disabled.home.packages == [];
        message = "disabled connection must not install anything";
      }
      {
        name = "default-browser-is-runtime-discovery";
        assertion = enabled.canix-toolbelt.browserConnection.browser == null;
        message = "must not silently pick Chromium";
      }
      {
        name = "native-operation-inventory";
        assertion = builtins.elem "snapshot" adapter.operations && !(builtins.elem "heap.snapshot" adapter.operations);
        message = "advertise implemented WebDriver operations only";
      }
      {
        name = "argv-not-shell";
        assertion = builtins.length adapter.command == 1;
        message = "host command must be an explicit executable argv";
      }
    ];
  }
