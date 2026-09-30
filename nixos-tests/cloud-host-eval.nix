{
  inputs,
  pkgs,
}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  # Public test key only; never used to enroll a production machine.
  key = (import "${pkgs.path}/nixos/tests/ssh-keys.nix" pkgs).snakeOilEd25519PublicKey;
  evaluate = system: modules:
    import "${pkgs.path}/nixos/lib/eval-config.nix" {
      inherit system;
      modules =
        [
          inputs.disko.nixosModules.disko
          ../modules/nixos/hosts/cloud-host.nix
          ./fixtures/cloud-host.nix
        ]
        ++ modules;
    };
  uefi = evaluate "x86_64-linux" [];
  bios = evaluate "x86_64-linux" [{canix-toolbelt.cloudHost.boot.mode = lib.mkForce "bios";}];
  arm = evaluate "aarch64-linux" [];
  static = evaluate "x86_64-linux" [
    {
      canix-toolbelt.cloudHost.network = {
        ipv4 = {
          method = "manual";
          addresses = ["192.0.2.10/32"];
          gateway = "192.0.2.1";
          gatewayOnLink = true;
          dns = ["192.0.2.53"];
        };
        ipv6 = {
          method = "manual";
          addresses = ["2001:db8::10/128"];
          gateway = "fe80::1";
          gatewayOnLink = true;
          dns = ["2001:db8::53"];
        };
      };
    }
  ];
  disabled = import "${pkgs.path}/nixos/lib/eval-config.nix" {
    system = "x86_64-linux";
    modules = [inputs.disko.nixosModules.disko ../modules/nixos/hosts/cloud-host.nix];
  };
  macOnly = evaluate "x86_64-linux" [
    {
      canix-toolbelt.cloudHost.network = {
        interfaceName = lib.mkForce null;
        macAddress = "02:00:00:00:00:01";
      };
    }
  ];
  keyFile = evaluate "x86_64-linux" [{canix-toolbelt.cloudHost.access.authorizedKeys = lib.mkForce ["${key}\n"];}];
  inheritedServerDefaults = evaluate "x86_64-linux" [
    {
      nix.settings = {
        max-jobs = lib.mkDefault 4;
        min-free = lib.mkDefault (50 * 1024 * 1024 * 1024);
        max-free = lib.mkDefault (100 * 1024 * 1024 * 1024);
      };
    }
  ];
  profile = static.config.networking.networkmanager.ensureProfiles.profiles.cloud-uplink;
  rejectsNetworkValue = family: field: value: let
    test = evaluate "x86_64-linux" [{canix-toolbelt.cloudHost.network.${family}.${field} = lib.mkForce value;}];
  in
    !(builtins.tryEval (builtins.deepSeq test.config.canix-toolbelt.cloudHost.network.${family}.${field} true)).success;
  invalid = module: message:
    builtins.any (a: !a.assertion && a.message == message)
    (evaluate "x86_64-linux" [module]).config.assertions;
  valid = system:
    builtins.all (a: a.assertion) system.config.assertions
    && builtins.isString system.config.system.build.toplevel.drvPath;
in
  mkEvalCheck {
    name = "cloud-host-eval";
    assertions = [
      {
        name = "complete-systems";
        assertion = builtins.all valid [uefi bios arm static macOnly];
        message = "UEFI, BIOS, ARM UEFI and routed static systems must satisfy all NixOS assertions";
      }
      {
        name = "mac-matched-uplink";
        assertion =
          macOnly.config.networking.networkmanager.ensureProfiles.profiles.cloud-uplink.ethernet.mac-address
          == "02:00:00:00:00:01"
          && !(macOnly.config.networking.networkmanager.ensureProfiles.profiles.cloud-uplink.connection ? interface-name);
        message = "A provider adapter must be able to bind the uplink without assuming an interface name";
      }
      {
        name = "reject-invalid-addresses";
        assertion =
          rejectsNetworkValue "ipv4" "addresses" ["999.0.0.1/24"]
          && rejectsNetworkValue "ipv4" "addresses" ["192.0.2.1/33"]
          && rejectsNetworkValue "ipv6" "addresses" ["2001:db8::1/129"]
          && rejectsNetworkValue "ipv6" "gateway" "2001:::1"
          && rejectsNetworkValue "ipv4" "gateway" "gateway.example.test";
        message = "Invalid addresses and prefixes must fail type checking";
      }
      {
        name = "boot-and-root";
        assertion =
          uefi.config.boot.loader.systemd-boot.enable
          && !uefi.config.boot.loader.efi.canTouchEfiVariables
          && uefi.config.fileSystems."/".fsType == "btrfs"
          && uefi.config.fileSystems."/boot".fsType == "vfat"
          && bios.config.boot.loader.grub.enable
          && builtins.elem "/dev/disk/by-id/fixture-root" bios.config.boot.loader.grub.devices
          && bios.config.disko.devices.disk.system.content.partitions.bios.type == "EF02"
          && !(bios.config.fileSystems ? "/boot");
        message = "Disko mounts and each bootloader must agree on firmware and explicit installation disk";
      }
      {
        name = "btrfs-rollback-boundaries";
        assertion =
          builtins.all (
            mount:
              uefi.config.fileSystems.${mount}.fsType
              == "btrfs"
              && builtins.elem "compress=zstd:3" uefi.config.fileSystems.${mount}.options
              && builtins.elem "noatime" uefi.config.fileSystems.${mount}.options
          ) ["/" "/nix" "/var/lib" "/var/log" "/etc/ssh" "/.snapshots"]
          && builtins.elem "subvol=@identity" uefi.config.fileSystems."/etc/ssh".options
          && uefi.config.fileSystems."/etc/ssh".neededForBoot
          && uefi.config.services.btrfs.autoScrub.fileSystems == ["/"]
          && uefi.config.services.snapper.configs.root.SUBVOLUME == "/"
          && uefi.config.services.snapper.configs.root.TIMELINE_LIMIT_DAILY == 7
          && uefi.config.services.snapper.configs.root.TIMELINE_LIMIT_YEARLY == 0;
        message = "State, store, logs and SSH identity must be separate from bounded root snapshots";
      }
      {
        name = "routed-uplink";
        assertion =
          profile.ipv4.address1
          == "192.0.2.10/32"
          && profile.ipv4.route1 == "0.0.0.0/0,192.0.2.1"
          && profile.ipv4.route1_options == "onlink=true"
          && profile.ipv6.route1 == "::/0,fe80::1"
          && profile.ipv6.route1_options == "onlink=true";
        message = "Host-prefix addresses need an explicit on-link default route for both families";
      }
      {
        name = "key-only-access";
        assertion =
          !uefi.config.services.openssh.settings.PasswordAuthentication
          && !uefi.config.services.openssh.settings.KbdInteractiveAuthentication
          && uefi.config.services.openssh.settings.PermitRootLogin == "prohibit-password"
          && uefi.config.users.users.root.openssh.authorizedKeys.keys == [key]
          && keyFile.config.users.users.root.openssh.authorizedKeys.keys == [key];
        message = "The base host must permit key-only administration before a tunnel exists";
      }
      {
        name = "small-host-policy";
        assertion =
          uefi.config.nix.settings.max-jobs
          == 0
          && uefi.config.nix.settings.min-free == 256 * 1024 * 1024
          && uefi.config.nix.settings.max-free == 1024 * 1024 * 1024
          && uefi.config.networking.networkmanager.enable
          && !uefi.config.networking.dhcpcd.enable
          && !uefi.config.systemd.network.enable
          && !uefi.config.services.caddy.enable
          && !uefi.config.services.haproxy.enable;
        message = "A plain cloud VM must use small-host defaults and work independently of the gateway role";
      }
      {
        name = "override-large-server-defaults";
        assertion =
          inheritedServerDefaults.config.nix.settings.max-jobs
          == 0
          && inheritedServerDefaults.config.nix.settings.min-free == 256 * 1024 * 1024
          && inheritedServerDefaults.config.nix.settings.max-free == 1024 * 1024 * 1024;
        message = "Enabling the cloud profile must override generic server build/GC defaults without mkForce";
      }
      {
        name = "disabled-is-inert";
        assertion =
          disabled.config.disko.devices.disk
          == {}
          && !(disabled.config.networking.networkmanager.ensureProfiles.profiles ? cloud-uplink)
          && !disabled.config.services.openssh.enable
          && !disabled.config.services.btrfs.autoScrub.enable
          && disabled.config.services.snapper.configs == {};
        message = "Importing the configurator must not enable a cloud host";
      }
      {
        name = "reject-missing-access";
        assertion =
          invalid {canix-toolbelt.cloudHost.access.authorizedKeys = lib.mkForce [];}
          "cloudHost: at least one administrative SSH public key is required.";
        message = "An inaccessible cloud host must fail evaluation";
      }
      {
        name = "reject-incomplete-static";
        assertion =
          invalid {canix-toolbelt.cloudHost.network.ipv4.method = "manual";}
          "cloudHost: manual ipv4 requires addresses, a gateway and DNS servers.";
        message = "Manual networking must not accept incomplete connectivity";
      }
      {
        name = "reject-missing-nic";
        assertion =
          invalid {canix-toolbelt.cloudHost.network.interfaceName = lib.mkForce null;}
          "cloudHost: select the uplink by interfaceName, macAddress, or both.";
        message = "The configurator must not silently assume a provider interface name";
      }
      {
        name = "reject-password-override";
        assertion =
          invalid {services.openssh.settings.PasswordAuthentication = lib.mkForce true;}
          "cloudHost: SSH administration must remain key-only.";
        message = "A cloud install profile must reject temporary password authentication";
      }
      {
        name = "reject-arm-bios";
        assertion =
          builtins.any (a: !a.assertion && a.message == "cloudHost: BIOS boot requires x86_64-linux.")
          (evaluate "aarch64-linux" [{canix-toolbelt.cloudHost.boot.mode = lib.mkForce "bios";}]).config.assertions;
        message = "Unsupported architecture and firmware combinations must fail before installation";
      }
    ];
  }
