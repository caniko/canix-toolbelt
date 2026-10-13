{
  pkgs,
  inputs,
}: let
  inherit
    ((import "${pkgs.path}/nixos/lib/eval-config.nix" {
      system = pkgs.stdenv.hostPlatform.system;
      modules = [
        inputs.home-manager.nixosModules.home-manager
        ../modules/nixos/networking/direct-network.nix
        {
          users.users.operator = {
            isNormalUser = true;
            uid = 1000;
          };
          systemd.slices.canix-background.sliceConfig.MemoryMax = "4G";
          systemd.services.backup.serviceConfig = {
            User = "operator";
            ExecStart = "${pkgs.coreutils}/bin/true";
            ExecStartPre = "${pkgs.coreutils}/bin/true";
            Slice = "canix-background.slice";
            MemoryMax = "1G";
          };
          systemd.services.rooted.serviceConfig = {
            RootDirectory = "/srv/rooted";
            User = "nobody";
            ExecStart = "/bin/true";
            ExecStartPre = ["/bin/test -e /inside-root"];
            ExecStartPost = "/bin/test -e /inside-root";
            BindReadOnlyPaths = ["/srv/rooted-data:/data"];
            MemoryMax = "64M";
          };
          systemd.services.rooted.unitConfig = {
            StartLimitIntervalSec = "30s";
            StartLimitBurst = 2;
          };
          systemd.services.imaged.serviceConfig = {
            RootImage = "/srv/root.img";
            ExecStart = "/bin/true";
            ExecStartPre = ["/bin/test -e /inside-image"];
          };
          systemd.services.host-controls.serviceConfig = {
            RootImage = "/srv/unprepared.img";
            RootDirectoryStartOnly = true;
            ExecStart = "/bin/true";
            ExecStartPre = "${pkgs.coreutils}/bin/true";
          };
          systemd.services.reset.serviceConfig = {
            ExecStart = "${pkgs.coreutils}/bin/true";
            ExecStartPre = ["/bin/false" "" "/bin/false" " " "/bin/true"];
          };
          systemd.services.scalar-reset.serviceConfig = {
            ExecStart = "${pkgs.coreutils}/bin/true";
            ExecStartPre = "";
          };
          home-manager.users.operator = {
            home.stateVersion = "24.11";
            systemd.user.services.backend.Service = {
              ExecStart = "${pkgs.coreutils}/bin/true";
              ExecStartPre = "${pkgs.coreutils}/bin/true";
              Slice = "app-amc.slice";
              MemoryMax = "2G";
            };
            systemd.user.services.backend-list.Service = {
              ExecStart = "${pkgs.coreutils}/bin/true";
              ExecStartPre = ["${pkgs.coreutils}/bin/true" "${pkgs.coreutils}/bin/false"];
            };
            systemd.user.services.desktop.Service = {
              ExecStart = "${pkgs.coreutils}/bin/true";
              ExecStartPre = "${pkgs.coreutils}/bin/true";
            };
            systemd.user.services.rooted.Service = {
              RootDirectory = "/srv/user-root";
              ExecStart = "/bin/true";
              ExecStartPre = "/bin/test -e /inside-root";
              ExecStartPost = "/bin/test -e /inside-root";
              BindReadOnlyPaths = ["/srv/user-data:/data"];
              MemoryMax = "64M";
              Restart = "on-failure";
            };
            systemd.user.services.imaged.Service = {
              RootImage = "/srv/user-root.img";
              ExecStart = "/bin/true";
              ExecStartPre = ["/bin/test -e /inside-image"];
            };
            systemd.user.services.host-controls.Service = {
              RootImage = "/srv/user-unprepared.img";
              RootDirectoryStartOnly = "yes";
              ExecStart = "/bin/true";
              ExecStartPre = "${pkgs.coreutils}/bin/true";
            };
            systemd.user.services.reset.Service = {
              ExecStart = "${pkgs.coreutils}/bin/true";
              ExecStartPre = ["/bin/false" "" "/bin/false" " " "/bin/true"];
            };
            systemd.user.services.scalar-reset.Service = {
              ExecStart = "${pkgs.coreutils}/bin/true";
              ExecStartPre = "";
            };
            systemd.user.slices.app-amc.Slice.MemoryMax = "3G";
          };
          canix-toolbelt.networking.directNetwork = {
            enable = true;
            package = pkgs.hello;
            systemServices.backup = "canix-background.slice";
            systemServices.rooted = "canix-background.slice";
            systemServices.imaged = "canix-background.slice";
            systemServices.host-controls = "canix-background.slice";
            systemServices.reset = "canix-background.slice";
            systemServices.scalar-reset = "canix-background.slice";
            users.operator = {
              services.backend = "app-amc.slice";
              services.backend-list = "app-amc.slice";
              services.rooted = "app-amc.slice";
              services.imaged = "app-amc.slice";
              services.host-controls = "app-amc.slice";
              services.reset = "app-amc.slice";
              services.scalar-reset = "app-amc.slice";
              slices = ["agent-tools.slice"];
            };
            directInterfaces = ["wg-home"];
            dnsZones."vpn.example.test" = "10.123.0.1";
          };
        }
      ];
    }))
    config
    ;
  home = config.home-manager.users.operator;
  # Decode a fixture projection, rather than using executable store-context
  # strings as a dependency-free runtime input to builtins.fromJSON.
  policy = builtins.fromJSON (builtins.unsafeDiscardStringContext config.environment.etc."direct-network-policy.json".text);
  # Match systemd's effective command list, not just the unparsed Nix list.
  effective = pkgs.lib.foldl' (commands: command:
    if pkgs.lib.trim command == ""
    then []
    else commands ++ [command]) [];
in
  # An empty list emits no unit directive; inspect the rendered reset too.
  assert pkgs.lib.hasInfix "CapabilityBoundingSet=\n" config.systemd.units."direct-network-dns.service".text;
  assert config.users.users.direct-network-dns.isSystemUser;
  assert config.users.users.direct-network-dns.group == "direct-network-dns";
  assert config.systemd.services.direct-network-dns.serviceConfig.User == "direct-network-dns";
  assert config.systemd.services.direct-network-dns.serviceConfig.Group == "direct-network-dns";
  assert config.systemd.services.direct-network-dns.serviceConfig.NoNewPrivileges;
  assert config.systemd.services.direct-network-dns.serviceConfig.ExecStartPre == "+${pkgs.hello}/bin/canix-toolbelt-direct-network ready --routes-only";
  assert pkgs.lib.hasInfix "--user=direct-network-dns --group=direct-network-dns" config.systemd.services.direct-network-dns.serviceConfig.ExecStart;
  assert config.systemd.services.direct-network.serviceConfig.CapabilityBoundingSet == ["CAP_DAC_READ_SEARCH" "CAP_NET_ADMIN" "CAP_SYS_PTRACE"];
  assert config.systemd.services.nscd.environment.NSNCD_IGNORE_HOSTS == "true";
  assert pkgs.lib.hasInfix "enable-cache hosts no" config.services.nscd.config;
  assert config.systemd.services.backup.serviceConfig.Slice == "canix-background-direct.slice";
  assert config.systemd.services.backup.serviceConfig.MemoryMax == "1G";
  assert config.systemd.services.backup.serviceConfig.ExecStartPre == ["+${pkgs.hello}/bin/canix-toolbelt-direct-network ready" "${pkgs.coreutils}/bin/true"];
  assert effective config.systemd.services.reset.serviceConfig.ExecStartPre == ["+${pkgs.hello}/bin/canix-toolbelt-direct-network ready" "/bin/true"];
  assert effective config.systemd.services.scalar-reset.serviceConfig.ExecStartPre == ["+${pkgs.hello}/bin/canix-toolbelt-direct-network ready"];
  assert pkgs.lib.hasInfix (builtins.unsafeDiscardStringContext "ExecStartPre= \nExecStartPre=+${pkgs.hello}/bin/canix-toolbelt-direct-network ready\nExecStartPre=/bin/true\n") config.systemd.units."reset.service".text;
  assert config.systemd.services.rooted.serviceConfig.ExecStartPre == ["+${pkgs.hello}/bin/canix-toolbelt-direct-network ready" "/bin/test -e /inside-root"];
  assert config.systemd.services.imaged.serviceConfig.ExecStartPre == ["+${pkgs.hello}/bin/canix-toolbelt-direct-network ready" "/bin/test -e /inside-image"];
  assert config.systemd.services.rooted.serviceConfig.RootDirectory == "/srv/rooted";
  assert config.systemd.services.imaged.serviceConfig.RootImage == "/srv/root.img";
  assert !(config.systemd.services.rooted.serviceConfig.RootDirectoryStartOnly or false);
  assert config.systemd.services.host-controls.serviceConfig.RootDirectoryStartOnly;
  assert config.systemd.services.host-controls.serviceConfig.ExecStartPre == ["+${pkgs.hello}/bin/canix-toolbelt-direct-network ready" "${pkgs.coreutils}/bin/true"];
  assert config.systemd.services.rooted.serviceConfig.User == "nobody";
  assert config.systemd.services.rooted.serviceConfig.ExecStartPost == "/bin/test -e /inside-root";
  assert config.systemd.services.rooted.serviceConfig.BindReadOnlyPaths == ["/srv/rooted-data:/data"];
  assert config.systemd.services.rooted.serviceConfig.MemoryMax == "64M";
  assert config.systemd.services.rooted.unitConfig.StartLimitIntervalSec == "30s";
  assert config.systemd.services.rooted.unitConfig.StartLimitBurst == 2;
  assert !(config.systemd.units ? "direct-network-admission-rooted.service");
  assert !(config.systemd.units ? "direct-network-admission-imaged.service");
  assert !(config.environment.etc ? "systemd/system/backup.service.d/50-direct-network-rooted-admission.conf");
  # The native manager owns the complete systemd/system tree. Nested etc
  # entries attempt to mkdir through its immutable generated directory symlink.
  assert !(config.environment.etc ? "systemd/system/rooted.service.d/50-direct-network-rooted-admission.conf");
  assert !(config.environment.etc ? "systemd/system/imaged.service.d/50-direct-network-rooted-admission.conf");
  assert config.systemd.slices.canix-background.sliceConfig.MemoryMax == "4G";
  assert home.systemd.user.services.backend.Service.Slice == "app-amc-direct.slice";
  assert home.systemd.user.services.backend.Service.MemoryMax == "2G";
  assert home.systemd.user.services.backend.Service.ExecStartPre == ["${pkgs.hello}/bin/canix-toolbelt-direct-network ready" "${pkgs.coreutils}/bin/true"];
  assert home.systemd.user.services.backend-list.Service.ExecStartPre == ["${pkgs.hello}/bin/canix-toolbelt-direct-network ready" "${pkgs.coreutils}/bin/true" "${pkgs.coreutils}/bin/false"];
  assert effective home.systemd.user.services.reset.Service.ExecStartPre == ["${pkgs.hello}/bin/canix-toolbelt-direct-network ready" "/bin/true"];
  assert effective home.systemd.user.services.scalar-reset.Service.ExecStartPre == ["${pkgs.hello}/bin/canix-toolbelt-direct-network ready"];
  assert home.systemd.user.services.rooted.Service.ExecStartPre == ["${pkgs.hello}/bin/canix-toolbelt-direct-network ready" "/bin/test -e /inside-root"];
  assert home.systemd.user.services.imaged.Service.ExecStartPre == ["${pkgs.hello}/bin/canix-toolbelt-direct-network ready" "/bin/test -e /inside-image"];
  assert home.systemd.user.services.rooted.Service.RootDirectory == "/srv/user-root";
  assert home.systemd.user.services.imaged.Service.RootImage == "/srv/user-root.img";
  assert home.systemd.user.services.rooted.Service.ExecStartPost == "/bin/test -e /inside-root";
  assert home.systemd.user.services.rooted.Service.BindReadOnlyPaths == ["/srv/user-data:/data"];
  assert home.systemd.user.services.rooted.Service.MemoryMax == "64M";
  assert home.systemd.user.services.rooted.Service.Restart == "on-failure";
  assert home.systemd.user.services.host-controls.Service.RootDirectoryStartOnly == "yes";
  assert home.systemd.user.services.host-controls.Service.ExecStartPre == ["${pkgs.hello}/bin/canix-toolbelt-direct-network ready" "${pkgs.coreutils}/bin/true"];
  assert !(home.xdg.configFile ? "systemd/user/host-controls.service.d/50-direct-network-rooted-admission.conf");
  assert !(home.xdg.configFile ? "systemd/user/backend.service.d/50-direct-network-rooted-admission.conf");
  assert home.xdg.configFile."systemd/user/imaged.service.d/50-direct-network-rooted-admission.conf".source == home.xdg.configFile."systemd/user/rooted.service.d/50-direct-network-rooted-admission.conf".source;
  assert home.systemd.user.services.desktop.Service.ExecStartPre == "${pkgs.coreutils}/bin/true";
  assert home.systemd.user.slices.app-amc.Slice.MemoryMax == "3G";
  assert home.systemd.user.slices.app-amc-direct.Unit.Description != "";
  assert builtins.elem "direct-network-agent-tools.service" home.systemd.user.services.backend.Unit.Requires;
  assert builtins.elem "user.slice/user-1000.slice/user@1000.service/app.slice/app-amc.slice/app-amc-direct.slice" policy.cgroupPaths;
  assert builtins.elem "user.slice/user-1000.slice/user@1000.service/agent.slice/agent-tools.slice" policy.cgroupPaths;
  assert !(builtins.elem "user.slice/user-1000.slice" policy.cgroupPaths);
  assert !(builtins.elem "user.slice/user-1000.slice/user@1000.service/app.slice" policy.cgroupPaths);
  assert policy.allowedUsers == [1000];
  assert config.system.nssDatabases.hosts == ["files" "mymachines" "myhostname" "dns"];
    pkgs.runCommand "direct-network-eval" {} ''
      # Realise final /etc construction, not only the standalone drop-in file.
      units=${config.system.build.etc}/etc/systemd/system
      dropin="$units/rooted.service.d/50-direct-network-rooted-admission.conf"
      cmp "$dropin" "$units/imaged.service.d/50-direct-network-rooted-admission.conf"
      cmp "$dropin" ${home.xdg.configFile."systemd/user/rooted.service.d/50-direct-network-rooted-admission.conf".source}
      test -e "$units/rooted.service"
      test -e "$units/imaged.service"
      test ! -e "$units/backup.service.d/50-direct-network-rooted-admission.conf"
      test ! -e "$units/host-controls.service.d/50-direct-network-rooted-admission.conf"
      grep -Fx '[Service]' "$dropin"
      grep -Fx 'BindReadOnlyPaths=/run/direct-network' "$dropin"
      ! grep -Fx 'BindReadOnlyPaths=/nix/store' "$dropin"
      ! grep -Fx 'BindReadOnlyPaths=' "$dropin"
      while IFS= read -r path; do
        grep -Fx "BindReadOnlyPaths=$path" "$dropin"
      done < ${(pkgs.closureInfo {rootPaths = [pkgs.hello];})}/store-paths
      cp "$dropin" "$out"
    ''
