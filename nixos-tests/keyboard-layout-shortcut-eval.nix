{pkgs}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  module = ../modules/home/desktop/keyboard-layout-shortcut.nix;
  cosmicLib = {
    cosmic.mkRON = _kind: value: value;
  };
  mkEvaluated = {layout, shortcut ? null}:
    lib.evalModules {
      specialArgs = {inherit cosmicLib;};
      modules = [
        {
          options = {
            wayland.desktopManager.cosmic.compositor.xkb_config.layout = lib.mkOption {
              type = lib.types.str;
              default = "";
            };
            wayland.desktopManager.cosmic.shortcuts = lib.mkOption {
              type = lib.types.listOf lib.types.anything;
              default = [];
            };
          };
        }
        module
        {
          wayland.desktopManager.cosmic.compositor.xkb_config.layout = layout;
          canix-toolbelt.keyboard-layout-shortcut =
            {enable = true;}
            // lib.optionalAttrs (shortcut != null) {inherit shortcut;};
        }
      ];
    };
  one = mkEvaluated {layout = "us";};
  many = mkEvaluated {layout = "us,no";};
  custom = mkEvaluated {
    layout = "us,no,de";
    shortcut = "Alt+Shift";
  };
  shortcut = evaluated: builtins.head evaluated.config.wayland.desktopManager.cosmic.shortcuts;
in
  mkEvalCheck {
    name = "keyboard-layout-shortcut-eval";
    resultMessage = "Keyboard-layout shortcut evaluation passed";
    assertions = [
      {
        name = "single-layout-disabled";
        assertion = one.config.wayland.desktopManager.cosmic.shortcuts == [];
        message = "one layout must not add a switch shortcut";
      }
      {
        name = "multiple-layouts-enabled";
        assertion = shortcut many == {
          action = {
            value = ["InputSourceSwitch"];
            variant = "System";
          };
          description = "Switch keyboard layout";
          key = "Super+Delete";
        };
        message = "multiple layouts must add the default switch shortcut";
      }
      {
        name = "custom-shortcut";
        assertion = (shortcut custom).key == "Alt+Shift";
        message = "the switch shortcut must be overrideable";
      }
    ];
  }
