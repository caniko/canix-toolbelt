{pkgs}: let
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  inherit (pkgs) lib;
  inherit ((import ../lib {inherit lib;})) hostSelection;

  evaluated = lib.evalModules {
    modules = [
      ../flake-modules/host-selection.nix
      {
        canix-toolbelt.host-selection.bar.enable = false;
        canix-toolbelt.host-selection.atlas.enable = true;
      }
    ];
  };
  policy = evaluated.config.canix-toolbelt.host-selection;
  fixture = {
    atlas = {value = "kept";};
    bar = {value = throw "disabled host payload was forced";};
  };
  disabled = hostSelection.filterEnabled policy fixture;
  restored = hostSelection.filterEnabled (policy // {bar.enable = true;}) fixture;
  unknown = hostSelection.unknownHosts policy ["atlas" "bar"];
in
  mkEvalCheck {
    name = "host-selection-eval";
    resultMessage = "host selection defaults, filtering, and restoration passed";
    assertions = [
      {
        name = "typed-disable";
        assertion = policy.bar.enable == false;
        message = "explicit host disable must be preserved by the flake option";
      }
      {
        name = "default-enable";
        assertion = policy.atlas.enable == true;
        message = "explicitly enabled hosts must remain enabled";
      }
      {
        name = "lazy-filter";
        assertion = builtins.deepSeq disabled (builtins.attrNames disabled == ["atlas"]);
        message = "disabled host payloads must not be forced by filtering";
      }
      {
        name = "restore";
        assertion = builtins.attrNames restored == ["atlas" "bar"];
        message = "setting enable back to true must restore the host";
      }
      {
        name = "known-hosts";
        assertion = unknown == [];
        message = "the policy validator must accept known host names";
      }
      {
        name = "unknown-hosts";
        assertion = hostSelection.unknownHosts (policy // {typo.enable = false;}) ["atlas" "bar"] == ["typo"];
        message = "the policy validator must report misspelled host names";
      }
    ];
  }
