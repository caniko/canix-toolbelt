{
  pkgs,
  homeManager,
}: let
  homeDirectory = "/home/roborev-fixture";
  root = "${homeDirectory}/fixture";
  arguments = {
    inherit pkgs homeManager homeDirectory;
    agentCommand = "${root}/agent $literal %pct \"quote\" \\back";
    environmentFile = "${root}/environment $literal %pct \"quote\" \\back";
    requiredFile = "${root}/required";
  };
  fixtures = [
    (import ./unit.nix arguments)
    (import ./unit.nix (arguments // {settings.max_workers = 3;}))
    (import ./unit.nix (arguments
      // {
        settings.max_workers = 3;
        extraPackages = [pkgs.hello];
      }))
    (import ./unit.nix (arguments
      // {
        settings.max_workers = 3;
        extraPackages = [pkgs.hello];
        recorderRevision = "changed-package";
      }))
  ];
  projection = pkgs.writeText "roborev-unit-fixtures.json" (builtins.toJSON fixtures);
  checker = pkgs.writeText "roborev-unit-guest.py" (builtins.readFile ./unit-guest.py);
in
  assert (builtins.elemAt fixtures 0).configFile != (builtins.elemAt fixtures 1).configFile;
  assert (builtins.elemAt fixtures 0).wrappedPackage == (builtins.elemAt fixtures 1).wrappedPackage;
  assert (builtins.elemAt fixtures 1).configFile == (builtins.elemAt fixtures 2).configFile;
  assert (builtins.elemAt fixtures 1).wrappedPackage != (builtins.elemAt fixtures 2).wrappedPackage;
  assert (builtins.elemAt fixtures 2).configFile == (builtins.elemAt fixtures 3).configFile;
  assert (builtins.elemAt fixtures 2).wrappedPackage != (builtins.elemAt fixtures 3).wrappedPackage;
    pkgs.testers.runNixOSTest {
      name = "roborev-generated-unit";
      nodes.machine = {
        users.users.roborev-fixture = {
          isNormalUser = true;
          uid = 1000;
          home = homeDirectory;
        };
        environment.systemPackages = [pkgs.python3 pkgs.systemd pkgs.util-linux];
        environment.etc."roborev-fixture".text = "isolated-nixos-test\n";
        # Reference the exact HM artifacts and full generations in the VM closure.
        environment.etc."roborev-unit-fixtures.json".source = projection;
        virtualisation = {
          memorySize = 2048;
          cores = 2;
          diskSize = 4096;
        };
      };
      testScript = ''
        start_all()
        machine.wait_for_unit("multi-user.target")
        machine.succeed("systemctl start user@1000.service")
        machine.wait_for_unit("user@1000.service")
        try:
            machine.succeed("runuser -u roborev-fixture -- env HOME=${homeDirectory} XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus python3 ${checker} /etc/roborev-unit-fixtures.json ${homeDirectory}/receipts")
        finally:
            machine.succeed("systemctl stop user@1000.service")
            machine.fail("pgrep -u 1000")
            if machine.execute("test -d ${homeDirectory}/receipts")[0] == 0:
                machine.copy_from_machine("${homeDirectory}/receipts")
                # Preserve the guest's causal assertion in the bounded build
                # tail even when the test output cannot be realized.
                if machine.execute("test -f ${homeDirectory}/receipts/unit-results.json")[0] == 0:
                    print(machine.succeed("cat ${homeDirectory}/receipts/unit-results.json"))
      '';
    }
