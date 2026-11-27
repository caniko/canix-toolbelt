{pkgs}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  base = {
    options = {
      age.enable = lib.mkEnableOption "agenix";
      assertions = lib.mkOption {
        type = lib.types.listOf lib.types.attrs;
        default = [];
      };
      systemd.user.services = lib.mkOption {
        type = lib.types.attrsOf (lib.types.submodule {
          options = {
            Unit = lib.mkOption {
              type = lib.types.attrsOf lib.types.anything;
              default = {};
            };
            Service = lib.mkOption {
              type = lib.types.attrsOf lib.types.anything;
              default = {};
            };
          };
        });
        default = {};
      };
    };
  };

  evaluate = {
    enable,
    ageEnable,
    services ? {},
  }:
    (lib.evalModules {
      specialArgs = {inherit pkgs;};
      modules = [
        base
        ../modules/home/agenix-once.nix
        {
          age.enable = ageEnable;
          canix-toolbelt.agenix-once.enable = enable;
          systemd.user.services = services;
        }
      ];
    }).config;

  softDeps = evaluate {
    enable = true;
    ageEnable = true;
    services.good-unit.Unit = {
      Wants = ["agenix.service"];
      After = ["agenix.service"];
    };
  };

  hardDeps = evaluate {
    enable = true;
    ageEnable = true;
    services = {
      bad-requires.Unit.Requires = ["agenix.service"];
      bad-partof.Unit.PartOf = ["agenix.service"];
    };
  };

  disabled = evaluate {
    enable = false;
    ageEnable = true;
    services.bad-requires.Unit.Requires = ["agenix.service"];
  };

  ageDisabled = evaluate {
    enable = true;
    ageEnable = false;
    services.bad-requires.Unit.Requires = ["agenix.service"];
  };

  hardDepsFailed = lib.filter (a: !a.assertion) hardDeps.assertions;
  hardDepsMessage = lib.concatMapStringsSep "\n" (a: a.message) hardDepsFailed;
in
  mkEvalCheck {
    name = "agenix-once-eval";
    resultMessage = "agenix-once Home Manager module evaluation passed";
    assertions = [
      {
        name = "soft-deps-pass-and-set-remain-after-exit";
        assertion =
          softDeps.systemd.user.services.agenix.Service.RemainAfterExit
          == true
          && lib.all (a: a.assertion) softDeps.assertions;
        message = "enabled agenix-once with Wants/After must set RemainAfterExit and pass all assertions";
      }
      {
        name = "hard-deps-fail-assertion-and-name-offenders";
        assertion =
          hardDepsFailed
          != []
          && lib.hasInfix "bad-requires" hardDepsMessage
          && lib.hasInfix "bad-partof" hardDepsMessage;
        message = "hard dependencies on agenix.service must fail assertion and mention all offending units";
      }
      {
        name = "disabled-does-not-modify-agenix-or-fail";
        assertion =
          !(disabled.systemd.user.services ? agenix)
          && lib.all (a: a.assertion) disabled.assertions;
        message = "disabled agenix-once must not set RemainAfterExit or emit failing assertions";
      }
      {
        name = "age-disabled-does-not-modify-agenix-or-fail";
        assertion =
          !(ageDisabled.systemd.user.services ? agenix)
          && lib.all (a: a.assertion) ageDisabled.assertions;
        message = "agenix-once with age.enable = false must not set RemainAfterExit or emit failing assertions";
      }
    ];
  }
