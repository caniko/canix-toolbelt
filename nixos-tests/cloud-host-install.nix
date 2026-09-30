{
  inputs,
  pkgs,
  mode,
}: let
  system = import "${pkgs.path}/nixos/lib/eval-config.nix" {
    system = "x86_64-linux";
    modules = [
      inputs.disko.nixosModules.disko
      ../modules/nixos/hosts/cloud-host.nix
      ./fixtures/cloud-host.nix
      ({lib, ...}: {
        canix-toolbelt.cloudHost.boot.mode = lib.mkForce mode;
        environment.etc."cloud-host-generation".text = lib.mkDefault "base";
        specialisation.updated.configuration.environment.etc."cloud-host-generation".text = "updated";
        # Test cleanup without waiting for the production one-hour minimum age.
        services.snapper.configs.root.NUMBER_MIN_AGE = 0;
        disko.tests.extraChecks = ''
          machine.wait_for_unit("sshd.service")
          machine.succeed("test $(findmnt -n -o FSTYPE /) = btrfs")
          for mountpoint in ["/nix", "/var/lib", "/var/log", "/etc/ssh", "/.snapshots"]:
              machine.succeed(f"test $(findmnt -n -o FSTYPE --mountpoint {mountpoint}) = btrfs")
              machine.succeed(f"findmnt -n -o OPTIONS --mountpoint {mountpoint} | grep -q compress=zstd:3")
          # Match the unit's explicit config path, not OpenSSH's build-time
          # default under /nix/store.
          ssh_config = machine.succeed("sshd -T -f /etc/ssh/sshd_config").lower().splitlines()
          # OpenSSH 10.5 prints canonical-case directive names, older versions
          # print lowercase. Option spelling is case-insensitive.
          assert "passwordauthentication no" in ssh_config, ssh_config
          assert "kbdinteractiveauthentication no" in ssh_config, ssh_config
          machine.succeed("test -s /etc/ssh/ssh_host_ed25519_key")
          for directory in ["/tmp", "/var/tmp", "/home", "/srv", "/var"]:
              machine.fail(f"btrfs subvolume show {directory}")
          host_key = machine.succeed("sha256sum /etc/ssh/ssh_host_ed25519_key")
          base = machine.succeed("readlink -f /run/current-system").strip()
          # Disko's runner shares /nix/store read-only over 9p. Preserve the
          # installed root here; exercise real GC in a separate writable store
          # containing this complete generation during offline recovery below.
          machine.succeed(f"mkdir -p /nix/var/nix/gcroots; ln -s {base} /nix/var/nix/gcroots/cloud-host-checkpoint")
          machine.succeed("mkdir -p /var/lib/cloud-host-fixture; echo retained > /var/lib/cloud-host-fixture/state")
          machine.succeed("mkdir -p /var/lib/wireguard /var/lib/acme; echo fixture-key > /var/lib/wireguard/edge.key; echo fixture-acme-state > /var/lib/acme/state; chmod 600 /var/lib/wireguard/edge.key")
          machine.succeed("echo before > /root/rollback-fixture")
          snapshot = machine.succeed("snapper --no-dbus -c root create --print-number --cleanup-algorithm number").strip()
          machine.succeed("echo after > /root/rollback-fixture")
          machine.succeed(f"grep -qx before /.snapshots/{snapshot}/snapshot/root/rollback-fixture")
          # Btrfs snapshots are non-recursive: identities and mutable service
          # data remain on their own mounted subvolumes when root is restored.
          machine.fail(f"test -e /.snapshots/{snapshot}/snapshot/etc/ssh/ssh_host_ed25519_key")
          machine.fail(f"test -e /.snapshots/{snapshot}/snapshot/var/lib/cloud-host-fixture/state")
          machine.fail(f"test -e /.snapshots/{snapshot}/snapshot/var/lib/wireguard/edge.key")
          machine.succeed(f"snapper --no-dbus -c root undochange {snapshot}..0 /root/rollback-fixture")
          machine.succeed("grep -qx before /root/rollback-fixture")
          machine.succeed(f"{base}/specialisation/updated/bin/switch-to-configuration test")
          machine.succeed("grep -qx updated /etc/cloud-host-generation")
          machine.succeed(f"{base}/bin/switch-to-configuration test")
          machine.succeed("grep -qx base /etc/cloud-host-generation")
          # Repeat normal activation, never repartition the installed system.
          machine.succeed("/run/current-system/bin/switch-to-configuration test")
          machine.succeed("/run/current-system/bin/switch-to-configuration test")
          assert machine.succeed("sha256sum /etc/ssh/ssh_host_ed25519_key") == host_key
          machine.reboot()
          # Disko starts this guest with the driver's default -no-reboot.
          # Restart QEMU on the same installed disk after the clean reboot exit.
          machine.wait_for_shutdown()
          machine.start()
          machine.wait_for_unit("sshd.service")
          assert machine.succeed("sha256sum /etc/ssh/ssh_host_ed25519_key") == host_key
          machine.succeed("grep -qx retained /var/lib/cloud-host-fixture/state")
          machine.succeed("grep -qx fixture-key /var/lib/wireguard/edge.key; grep -qx fixture-acme-state /var/lib/acme/state")
          assert machine.succeed("readlink /nix/var/nix/gcroots/cloud-host-checkpoint").strip() == base
          machine.succeed("grep -qx before /root/rollback-fixture")
          machine.succeed(f"snapper --no-dbus -c root delete {snapshot}")
          numbered = [machine.succeed("snapper --no-dbus -c root create --print-number --cleanup-algorithm number").strip() for _ in range(7)]
          machine.succeed("snapper --no-dbus -c root cleanup number")
          for number in numbered[:2]:
              machine.fail(f"test -d /.snapshots/{number}")
          for number in numbered[2:]:
              machine.succeed(f"test -d /.snapshots/{number}/snapshot")

          # Exercise the documented whole-root procedure from the independent
          # installer, with the installed disk unmounted. Never replace a live
          # mounted root or assume snapper's default-subvolume flag selects it.
          machine.succeed("echo checkpoint > /root/whole-root-checkpoint")
          checkpoint = machine.succeed("snapper --no-dbus -c root create --print-number").strip()
          uuid = machine.succeed("findmnt -n -o UUID /").strip()
          machine.succeed("echo damaged > /root/whole-root-checkpoint; touch /root/after-checkpoint")
          machine.shutdown()
          recovery = next(m for m in driver.machines_qemu if m.name == "machine")
          recovery.start()
          recovery.succeed(f"mkdir -p /mnt/recovery; mount -o subvolid=5 /dev/disk/by-uuid/{uuid} /mnt/recovery")
          # The installer registered this generation before first boot. Copy
          # its entire closure into an isolated writable store on @nix, so this
          # test never asks GC to modify the runner's shared store.
          store_root = "/mnt/recovery/@nix/closure-test"
          store = f"local?root={store_root}"
          # These are build-local fixture paths, not signed cache downloads;
          # copy the installer's already registered closure into its test store.
          recovery.succeed(f"nix --extra-experimental-features nix-command copy --no-check-sigs --to '{store}' {base}")
          recovery.succeed(f"mkdir -p {store_root}/nix/var/nix/gcroots; ln -s {base} {store_root}/nix/var/nix/gcroots/checkpoint")
          recovery.succeed("echo collect-me > /tmp/unreferenced-cloud-fixture")
          garbage = recovery.succeed(f"nix-store --store '{store}' --add /tmp/unreferenced-cloud-fixture").strip()
          recovery.succeed(f"nix-store --store '{store}' --gc")
          recovery.fail(f"test -e {store_root}{garbage}")
          live = recovery.succeed(f"nix-store --store '{store}' --gc --print-live").splitlines()
          assert base in live and len(live) > 1, live
          recovery.succeed(f"nix-store --store '{store}' --verify --check-contents")
          recovery.succeed("mv /mnt/recovery/@root /mnt/recovery/@root-before-recovery")
          recovery.succeed(f"btrfs subvolume snapshot /mnt/recovery/@snapshots/{checkpoint}/snapshot /mnt/recovery/@root")
          recovery.succeed("sync; umount /mnt/recovery")
          recovery.shutdown()
          machine.start()
          machine.wait_for_unit("sshd.service")
          machine.succeed("grep -qx checkpoint /root/whole-root-checkpoint")
          machine.fail("test -e /root/after-checkpoint")
          for directory in ["/tmp", "/var/tmp", "/home", "/srv", "/var"]:
              machine.succeed(f"touch {directory}/recovery-write-check; rm {directory}/recovery-write-check")
          machine.wait_for_unit("dbus.service")
          machine.wait_for_unit("NetworkManager.service")
          machine.succeed(f"{base}/bin/switch-to-configuration test")
          machine.succeed("grep -qx base /etc/cloud-host-generation")
          assert machine.succeed("sha256sum /etc/ssh/ssh_host_ed25519_key") == host_key
          machine.succeed("grep -qx retained /var/lib/cloud-host-fixture/state; grep -qx fixture-key /var/lib/wireguard/edge.key; grep -qx fixture-acme-state /var/lib/acme/state")
          assert machine.succeed("readlink /nix/var/nix/gcroots/cloud-host-checkpoint").strip() == base
          machine.succeed(f"test -x /nix/closure-test{base}/bin/switch-to-configuration")
          assert machine.succeed("readlink /nix/closure-test/nix/var/nix/gcroots/checkpoint").strip() == base
        '';
      })
    ];
  };
in
  system.config.system.build.installTest
