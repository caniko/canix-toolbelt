{pkgs}: let
  inherit (pkgs) lib;
  operator = (import ../lib/systemd.nix {inherit lib;}).mkResumableOperator {
    inherit pkgs;
    name = "fixture-operator";
    stateDir = "/var/lib/fixture-operator";
    stages = [
      {name = "first"; unit = "fixture-first.service";}
      {name = "second"; unit = "fixture-second.service";}
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
in
  assert main.serviceConfig.Type == "notify";
  assert main.serviceConfig.RestartPreventExitStatus == "20";
  assert lib.elem "/var/lib/fixture-operator" main.serviceConfig.ReadWritePaths;
  assert cancel.serviceConfig.Type == "oneshot";
  assert resume.unitConfig.ConditionPathExists == "/var/lib/fixture-operator/requested";
    pkgs.runCommand "resumable-operator-eval" {} ''
      test -x ${controller}
      ${pkgs.bash}/bin/bash -n ${controller}
      definition_line=$(grep -n '^run_stage()' ${controller} | cut -d: -f1)
      call_line=$(grep -n '^run_stage first ' ${controller} | head -1 | cut -d: -f1)
      test "$definition_line" -lt "$call_line"
      touch $out
    ''
