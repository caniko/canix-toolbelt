{pkgs}:
pkgs.testers.nixosTest {
  name = "dev-oom-guard-protection";

  nodes.machine = {pkgs, ...}: let
    testAgent = pkgs.writeShellScriptBin "oom-test-root" ''
      prefix="$1"
      printf '%s\n' "$$" > "/tmp/$prefix-root.pid"
      ${pkgs.coreutils}/bin/sleep infinity &
      printf '%s\n' "$!" > "/tmp/$prefix-child.pid"
      wait
    '';
  in {
    imports = [../modules/nixos/services/dev-oom-guard.nix];

    users.users.guard = {
      isNormalUser = true;
      uid = 1000;
    };

    canix-toolbelt.services.devOomGuard = {
      enable = true;
      pollSeconds = 1;
      verifyEditorScopes = false;
      agents = [
        {
          name = "test-agent";
          cmdlineRegex = "oom-test-root";
        }
      ];
      protect = [
        {
          name = "test-root";
          cmdlineRegex = "oom-test-root";
          maxAdj = -1000;
        }
      ];
    };

    environment.systemPackages = [testAgent];
  };

  testScript = ''
    start_all()

    machine.succeed("loginctl enable-linger guard")
    machine.wait_for_unit("user@1000.service")
    machine.wait_until_succeeds(
        "systemctl --machine=guard@ --user is-active dev-oom-guard.service"
    )

    machine.succeed("runuser -u guard -- oom-test-root guard >/tmp/guard.log 2>&1 &")
    machine.succeed("oom-test-root root >/tmp/root.log 2>&1 &")
    machine.wait_until_succeeds("test -s /tmp/guard-root.pid -a -s /tmp/guard-child.pid -a -s /tmp/root-root.pid")

    guard_root = machine.succeed("tr -d '\\n' </tmp/guard-root.pid")
    guard_child = machine.succeed("tr -d '\\n' </tmp/guard-child.pid")
    root_root = machine.succeed("tr -d '\\n' </tmp/root-root.pid")

    machine.wait_until_succeeds(f'test "$(cat /proc/{guard_root}/oom_score_adj)" = -1000')
    machine.wait_until_succeeds(f'test "$(cat /proc/{guard_child}/oom_score_adj)" = 1000')
    machine.succeed(f'test "$(cat /proc/{root_root}/oom_score_adj)" = 0')
  '';
}
