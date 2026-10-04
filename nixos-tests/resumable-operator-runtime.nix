{pkgs}: let
  operator = (import ../lib/systemd.nix {inherit (pkgs) lib;}).mkResumableOperator {
    inherit pkgs;
    name = "fixture-operator";
    executionContract = "fixture-executable-argv-v1";
    stateDir = "/var/lib/fixture-operator";
    quiesceUnits = ["fixture-worker.service"];
    stages = [
      {
        name = "first";
        unit = "fixture-stage.service";
      }
    ];
    resumeOnBoot = true;
  };
in
  pkgs.testers.runNixOSTest {
    name = "toolbelt-packaged-operator";
    nodes.machine = {
      imports = [operator];
      systemd.tmpfiles.rules = ["d /var/lib/fixture-operator 0750 root root -"];
      systemd.services.fixture-worker.serviceConfig.ExecStart = "${pkgs.coreutils}/bin/sleep infinity";
      systemd.services.fixture-stage = {
        serviceConfig.Type = "oneshot";
        script = ''
          test -f /var/lib/fixture-operator/running
          if ${pkgs.systemd}/bin/systemctl is-active --quiet fixture-worker; then exit 1; fi
          echo stage >> /var/lib/fixture-operator/events
          sleep 5
        '';
      };
    };
    testScript = ''
      start_all()
      machine.wait_for_unit("multi-user.target")
      machine.succeed("systemctl start fixture-worker")
      for expected in (1, 2):
          machine.succeed("touch /var/lib/fixture-operator/requested; systemctl start fixture-operator")
          machine.succeed("test -f /var/lib/fixture-operator/running")
          machine.fail("systemctl is-active --quiet fixture-worker")
          machine.wait_until_succeeds("test ! -e /var/lib/fixture-operator/requested")
          machine.succeed("test ! -e /var/lib/fixture-operator/running")
          machine.wait_for_unit("fixture-worker")
          assert int(machine.succeed("wc -l < /var/lib/fixture-operator/events").strip()) == expected
      machine.succeed("test $(systemctl show fixture-operator -p ExecMainStatus --value) = 0")
    '';
  }
