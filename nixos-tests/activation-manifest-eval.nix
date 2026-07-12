{pkgs}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  optionStubs = {lib, ...}: {
    options = {
      assertions = lib.mkOption {
        type = lib.types.listOf lib.types.attrs;
        default = [];
      };
      environment.etc = lib.mkOption {
        type = lib.types.attrsOf (lib.types.submodule {
          options = {
            mode = lib.mkOption {type = lib.types.str; default = "0644";};
            text = lib.mkOption {type = lib.types.str; default = "";};
          };
        });
        default = {};
      };
    };
  };

  evaluated = lib.evalModules {
    modules = [
      optionStubs
      ../modules/nixos/activation-manifest.nix
      {
        canix-toolbelt.activation.contracts.openobserve = {
          enabled = true;
          owner = "canix:root/hosts/thething/server/openobserve.nix";
          rollout = "reconcile";
          requirements.password = {
            phase = "runtime";
            kind = "secret";
            producerId = "secret-manager:openobservePassword";
            check = {
              kind = "secret";
              path = "/run/agenix/openobservePassword";
              predicates = ["readable" "non-empty"];
            };
          };
          artifacts = [
            {
              id = "data";
              kind = "directory";
              path = "/var/lib/openobserve";
              persistence = "state";
            }
          ];
          healthChecks = [
            {
              name = "healthz";
              kind = "http";
              url = "http://127.0.0.1:5080/healthz";
              acceptedStatus = [200];
            }
          ];
        };
      }
    ];
  };
  manifest = builtins.fromJSON evaluated.config.environment.etc."canix/activation-contracts.json".text;
  contract = manifest.contracts.openobserve;
in
  mkEvalCheck {
    name = "activation-manifest-eval";
    resultMessage = "activation manifest preserves v2 metadata without secret values";
    assertions = [
      {
        name = "schema-version";
        assertion = manifest.schemaVersion == 2;
        message = "manifest must use the v2 schema";
      }
      {
        name = "contract-owner";
        assertion = contract.owner == "canix:root/hosts/thething/server/openobserve.nix";
        message = "manifest must preserve the contract owner";
      }
      {
        name = "runtime-check";
        assertion = contract.requirements.password.check.path == "/run/agenix/openobservePassword";
        message = "manifest must preserve the typed runtime check path";
      }
      {
        name = "state-artifact";
        assertion = (builtins.head contract.artifacts).persistence == "state";
        message = "manifest must preserve state-artifact persistence";
      }
      {
        name = "health-check";
        assertion = (builtins.head contract.healthChecks).url == "http://127.0.0.1:5080/healthz";
        message = "manifest must preserve the non-mutating health check";
      }
      {
        name = "no-secret-value";
        assertion = !(lib.hasInfix "password-value" (builtins.toJSON manifest));
        message = "manifest must not contain secret values";
      }
    ];
  }
