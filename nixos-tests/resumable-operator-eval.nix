{pkgs}: let
  inherit (pkgs) lib;
  operator = (import ../lib/systemd.nix {inherit lib;}).mkResumableOperator {
    inherit pkgs;
    name = "fixture-operator";
    executionContract = "fixture-package-and-arguments-v1";
    stateDir = "/var/lib/fixture-operator";
    stages = [
      {
        name = "first";
        unit = "fixture-first.service";
      }
      {
        name = "second";
        unit = "fixture-second.service";
      }
    ];
    maxAttempts = 2;
    retryDelays = ["1s"];
    quiesceUnits = ["fixture-worker.service"];
    resumeOnBoot = true;
  };
  main = operator.systemd.services.fixture-operator;
  cancel = operator.systemd.services.fixture-operator-cancel;
  resume = operator.systemd.services.fixture-operator-resume;
  controller = lib.head (lib.splitString " " main.serviceConfig.ExecStart);
  policy = builtins.elemAt (lib.splitString " " main.serviceConfig.ExecStart) 4;
  helper = (import ../lib/systemd.nix {inherit lib;}).mkResumableOperator;
  emptyContract = builtins.tryEval (builtins.deepSeq (helper {
      inherit pkgs;
      name = "unsafe-default";
      executionContract = "";
      stateDir = "/var/lib/unsafe-default";
      stages = [
        {
          name = "first";
          unit = "fixture-first.service";
        }
      ];
    })
    true);
in
  assert (builtins.functionArgs helper).executionContract == false;
  assert !emptyContract.success;
  assert main.serviceConfig.Type == "notify";
  assert main.serviceConfig.RestartPreventExitStatus == "20";
  assert lib.elem "/var/lib/fixture-operator" main.serviceConfig.ReadWritePaths;
  assert cancel.serviceConfig.Type == "oneshot";
  assert resume.unitConfig.ConditionPathExists == "/var/lib/fixture-operator/requested";
    pkgs.runCommand "resumable-operator-eval" {nativeBuildInputs = [pkgs.jq pkgs.python3];} ''
      test -x ${controller}
      ${controller} operator run --help | grep -F -- --config
      ${controller} operator cancel --help | grep -F -- --config
      jq -e '.contract_id == "fixture-package-and-arguments-v1" and
        .state_dir == "/var/lib/fixture-operator" and
        .stages == [{"name":"first","unit":"fixture-first.service"},
                    {"name":"second","unit":"fixture-second.service"}]' ${policy}
      python ${./test-operator-runtime.py} ${controller} ${policy}
      touch $out
    ''
