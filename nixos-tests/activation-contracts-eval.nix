{pkgs}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  optionStubs = {lib, ...}: {
    options.assertions = lib.mkOption {
      type = lib.types.listOf lib.types.attrs;
      default = [];
      description = "Minimal assertions option for the contract fixture.";
    };
  };

  blocked = lib.evalModules {
    modules = [
      optionStubs
      ../modules/nixos/activation-contracts.nix
      {
        canix-toolbelt.activation.contracts.demo = {
          enabled = true;
          requirements.source = {
            summary = "demo input is absent";
            producer = "test fixture";
            recovery = "provide the fixture input";
            validation = "run the contract check";
          };
        };
      }
    ];
  };

  deferred = lib.evalModules {
    modules = [
      optionStubs
      ../modules/nixos/activation-contracts.nix
      {
        canix-toolbelt.activation.contracts.demo = {
          enabled = true;
          requirements.runtime-file = {
            phase = "runtime";
            summary = "runtime file is created by activation";
          };
        };
      }
    ];
  };

  helperActivation = import ../lib/activation.nix {inherit lib;};
  helperContract = helperActivation.mkContract {
    name = "fixture.demo";
    requirements.source.summary = "demo input is absent";
  };
  helper = lib.evalModules {
    modules = [
      optionStubs
      ../modules/nixos/activation-contracts.nix
      (helperActivation.contractModule "demo" helperContract)
    ];
  };
  # Callers that predate contractModule strip the metadata themselves.
  legacy = lib.evalModules {
    modules = [
      optionStubs
      ../modules/nixos/activation-contracts.nix
      {
        inherit (helperContract) assertions;
        canix-toolbelt.activation.contracts.demo = lib.removeAttrs helperContract ["name" "assertions" "status" "declaredStatus"];
      }
    ];
  };

  ready = lib.evalModules {
    modules = [
      optionStubs
      ../modules/nixos/activation-contracts.nix
      {
        canix-toolbelt.activation.contracts.demo = {
          enabled = true;
          requirements.source = {
            satisfied = true;
            summary = "demo input is present";
          };
          artifacts = [
            {
              path = "/run/demo/config";
              sensitive = true;
            }
          ];
          units = ["demo.service"];
        };
      }
    ];
  };
in
  mkEvalCheck {
    name = "activation-contracts-eval";
    resultMessage = "activation contract states and assertions are correct";
    assertions = [
      {
        name = "blocked-status";
        assertion = blocked.config.canix-toolbelt.activation.contracts.demo.status == "blocked";
        message = "an unsatisfied evaluation requirement must block the contract";
      }
      {
        name = "blocked-assertion";
        assertion = builtins.length blocked.config.assertions == 1;
        message = "an unsatisfied evaluation requirement must produce one assertion";
      }
      {
        name = "deferred-status";
        assertion = deferred.config.canix-toolbelt.activation.contracts.demo.status == "deferred";
        message = "a runtime-only requirement must defer the contract";
      }
      {
        name = "deferred-no-evaluation-assertion";
        assertion = deferred.config.assertions == [];
        message = "runtime-only requirements must not fail evaluation";
      }
      {
        name = "ready-status";
        assertion = ready.config.canix-toolbelt.activation.contracts.demo.status == "ready";
        message = "satisfied requirements must make the contract ready";
      }
      {
        name = "helper-module";
        assertion =
          helper.config.canix-toolbelt.activation.contracts.demo.status
          == "blocked"
          && builtins.length helper.config.assertions == 2;
        message = "contractModule must declare the contract and carry its assertions";
      }
      {
        name = "legacy-caller";
        assertion = legacy.config.canix-toolbelt.activation.contracts.demo.status == "blocked";
        message = "a caller that strips mkContract's metadata itself must still evaluate";
      }
      {
        name = "sensitive-artifact";
        assertion = (builtins.head ready.config.canix-toolbelt.activation.contracts.demo.artifacts).sensitive;
        message = "contract artifacts must preserve sensitivity metadata";
      }
    ];
  }
