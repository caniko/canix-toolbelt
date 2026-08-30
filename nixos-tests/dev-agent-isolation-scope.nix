{pkgs}: let
  inherit (pkgs) lib;
  wrapper = (import ../lib/dev-agent-isolation.nix {inherit lib;}).mkScopeExecWrapper {
    inherit pkgs;
    name = "dev-agent-scope-test";
    slice = "dev-agents.slice";
    targetPath = "${pkgs.bash}/bin/bash";
  };
  asUser = "runuser -u tester -- env XDG_RUNTIME_DIR=/run/user/1000";
  run = "${asUser} ${wrapper}";
in
  pkgs.testers.nixosTest {
    name = "dev-agent-isolation-scope";

    nodes.machine = {
      imports = [../modules/nixos/services/dev-agent-isolation.nix];

      users.users.tester = {
        isNormalUser = true;
        uid = 1000;
      };

      canix-toolbelt.services.devAgentIsolation = {
        enable = true;
        memoryHigh = "24G";
        memoryMax = "32G";
        memorySwapMax = "8G";
      };
    };

    testScript = ''
      start_all()
      machine.succeed("loginctl enable-linger tester")
      machine.wait_for_unit("user@1000.service")

      machine.fail("test -e /etc/systemd/user/dev-agent-workloads.service")
      machine.succeed("test -e /etc/systemd/user/dev-agents.slice")
      machine.fail("${asUser} systemctl --user cat dev-agent-workloads.service")

      cgroup = machine.succeed("${run} -c 'cat /proc/self/cgroup'")
      assert "dev-agents.slice" in cgroup, cgroup
      assert ".scope" in cgroup, cgroup

      machine.succeed("printf hello | ${run} -c cat | grep -x hello")

      literal = machine.succeed("${run} -c 'printf %s \"$1\"' x '$notexpanded'")
      assert literal.strip() == "$notexpanded", literal

      machine.succeed("mkdir -p /tmp/cwdtest")
      cwd = machine.succeed("${asUser} bash -c 'cd /tmp/cwdtest && ${wrapper} -c pwd'")
      assert cwd.strip() == "/tmp/cwdtest", cwd

      machine.succeed("bash -c '${run} -c \"exit 17\"; test $? -eq 17'")
    '';
  }
