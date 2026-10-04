{
  pkgs,
  modules,
}:
pkgs.testers.runNixOSTest {
  name = "toolbelt-harbor-db-compat-runtime";
  nodes.machine = {config, ...}: {
    imports = [modules.pg-backup modules.postgres-lifecycle];
    virtualisation.memorySize = 1024;
    services.postgresql = {
      enable = true;
      package = pkgs.postgresql_17;
      dataDir = "/var/lib/postgres/17";
    };
    services.harbor-db.postgresql = {
      enable = true;
      resource = "compat-fixture";
      switchAdoption.systemIdentifier = "12345";
    };
    environment.systemPackages = [pkgs.postgresql_17];
    environment.etc."fixture-adoption".source = config.system.preSwitchChecksScript;
    systemd.services.fixture-primary.serviceConfig = {
      User = "postgres";
      Group = "postgres";
      RuntimeDirectory = "postgresql";
      ExecStart = "${pkgs.postgresql_17}/bin/postgres -D /var/lib/postgres/17 -c unix_socket_directories=/run/postgresql";
    };
  };
  testScript = ''
    start_all()
    machine.wait_for_unit("multi-user.target")
    # The actual guarded startup must refuse to initialize absent storage.
    machine.fail("systemctl start postgresql")
    machine.succeed("systemctl stop postgresql; test ! -e /var/lib/postgres/17/PG_VERSION")
    machine.succeed("install -d -o postgres -g postgres -m 0700 /var/lib/postgres/17")
    machine.succeed("runuser -u postgres -- initdb -D /var/lib/postgres/17")
    identifier = machine.succeed("runuser -u postgres -- pg_controldata /var/lib/postgres/17 | sed -n 's/^Database system identifier: *//p'").strip()
    machine.succeed("systemctl start fixture-primary")
    machine.wait_until_succeeds("runuser -u postgres -- pg_isready -h /run/postgresql")
    machine.succeed("cp /etc/fixture-adoption /run/adoption; chmod +x /run/adoption")
    pid = machine.succeed("systemctl show fixture-primary -p MainPID --value").strip()
    machine.fail("/run/adoption /run/current-system test")
    assert machine.succeed("systemctl show fixture-primary -p MainPID --value").strip() == pid
    machine.succeed("test ! -e /var/lib/harbor-db/postgresql/identity.json")
    machine.succeed(f"sed -i 's/--system-identifier 12345/--system-identifier {identifier}/g' /run/adoption")
    # Unsafe live durability settings must refuse adoption as well.
    machine.succeed("runuser -u postgres -- psql -c 'ALTER SYSTEM SET fsync = off'")
    machine.succeed("runuser -u postgres -- psql -c 'SELECT pg_reload_conf()'")
    machine.wait_until_succeeds("test \"$(runuser -u postgres -- psql -Atqc 'SHOW fsync')\" = off")
    machine.fail("/run/adoption /run/current-system test")
    machine.succeed("test ! -e /var/lib/harbor-db/postgresql/identity.json")
    machine.succeed("runuser -u postgres -- psql -c 'ALTER SYSTEM RESET fsync'")
    machine.succeed("runuser -u postgres -- psql -c 'SELECT pg_reload_conf()'")
    machine.wait_until_succeeds("test \"$(runuser -u postgres -- psql -Atqc 'SHOW fsync')\" = on")
    for action in ("boot", "dry-activate"):
        machine.succeed(f"/run/adoption /run/current-system {action}")
        machine.succeed("test ! -e /var/lib/harbor-db/postgresql/identity.json")
    machine.succeed("/run/adoption /run/current-system test")
    machine.succeed("test -s /var/lib/harbor-db/postgresql/identity.json")
    machine.succeed("systemctl stop fixture-primary; systemctl reset-failed postgresql; systemctl start postgresql")
    machine.wait_for_unit("postgresql")
    machine.succeed("runuser -u postgres -- psql -c 'CREATE TABLE fixture (value int); INSERT INTO fixture VALUES (42)'")
    machine.succeed("systemctl restart postgresql")
    assert machine.succeed("runuser -u postgres -- psql -Atqc 'SELECT value FROM fixture'").strip() == "42"
    machine.succeed("systemctl stop postgresql; mv /var/lib/postgres/17 /var/lib/postgres/preserved")
    machine.fail("systemctl start postgresql")
    machine.succeed("systemctl stop postgresql; test ! -e /var/lib/postgres/17/PG_VERSION")
  '';
}
