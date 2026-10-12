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
            ExecStart = "/bin/true";
            ExecStartPre = ["/bin/test -e /inside-root"];
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
            systemd.user.slices.app-amc.Slice.MemoryMax = "3G";
          };
          canix-toolbelt.networking.directNetwork = {
            enable = true;
            package = pkgs.hello;
            systemServices.backup = "canix-background.slice";
            systemServices.rooted = "canix-background.slice";
            systemServices.imaged = "canix-background.slice";
            users.operator = {
              services.backend = "app-amc.slice";
              services.backend-list = "app-amc.slice";
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
in
  # An empty list emits no unit directive; inspect the rendered reset too.
  assert pkgs.lib.hasInfix "CapabilityBoundingSet=\n" config.systemd.units."direct-network-dns.service".text;
  assert config.systemd.services.nscd.environment.NSNCD_IGNORE_HOSTS == "true";
  assert pkgs.lib.hasInfix "enable-cache hosts no" config.services.nscd.config;
  assert config.systemd.services.backup.serviceConfig.Slice == "canix-background-direct.slice";
  assert config.systemd.services.backup.serviceConfig.MemoryMax == "1G";
  assert config.systemd.services.backup.serviceConfig.ExecStartPre == ["+${pkgs.hello}/bin/canix-toolbelt-direct-network ready" "${pkgs.coreutils}/bin/true"];
  assert config.systemd.services.rooted.serviceConfig.ExecStartPre == ["/bin/test -e /inside-root"];
  assert config.systemd.services.imaged.serviceConfig.ExecStartPre == ["/bin/test -e /inside-image"];
  assert config.systemd.services.rooted.serviceConfig.RootDirectory == "/srv/rooted";
  assert config.systemd.services.imaged.serviceConfig.RootImage == "/srv/root.img";
  assert !(config.systemd.services.rooted.serviceConfig.RootDirectoryStartOnly or false);
  assert config.systemd.services.rooted.serviceConfig.MemoryMax == "64M";
  assert config.systemd.services.rooted.unitConfig.StartLimitIntervalSec == "30s";
  assert config.systemd.services.rooted.unitConfig.StartLimitBurst == 2;
  assert builtins.elem "direct-network-admission-rooted.service" config.systemd.services.rooted.requires;
  assert builtins.elem "direct-network-admission-rooted.service" config.systemd.services.rooted.after;
  assert builtins.elem "direct-network-admission-imaged.service" config.systemd.services.imaged.requires;
  assert config.systemd.services.direct-network-admission-rooted.serviceConfig.Type == "oneshot";
  assert !(config.systemd.services.direct-network-admission-rooted.serviceConfig.RemainAfterExit or false);
  assert config.systemd.services.direct-network-admission-rooted.serviceConfig.ExecStart == "${pkgs.hello}/bin/canix-toolbelt-direct-network ready";
  assert config.systemd.services.direct-network-admission-rooted.unitConfig.StartLimitIntervalSec == 0;
  assert pkgs.lib.hasInfix "StartLimitIntervalSec=0\n" config.systemd.units."direct-network-admission-rooted.service".text;
  assert config.systemd.slices.canix-background.sliceConfig.MemoryMax == "4G";
  assert home.systemd.user.services.backend.Service.Slice == "app-amc-direct.slice";
  assert home.systemd.user.services.backend.Service.MemoryMax == "2G";
  assert home.systemd.user.services.backend.Service.ExecStartPre == ["${pkgs.hello}/bin/canix-toolbelt-direct-network ready" "${pkgs.coreutils}/bin/true"];
  assert home.systemd.user.services.backend-list.Service.ExecStartPre == ["${pkgs.hello}/bin/canix-toolbelt-direct-network ready" "${pkgs.coreutils}/bin/true" "${pkgs.coreutils}/bin/false"];
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
    pkgs.writeText "direct-network-eval" "Explicit cgroups retain all resource ancestors; desktop DNS and traffic remain separate.\n"
