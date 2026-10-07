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
            Slice = "canix-background.slice";
            MemoryMax = "1G";
          };
          home-manager.users.operator = {
            home.stateVersion = "24.11";
            systemd.user.services.backend.Service = {
              ExecStart = "${pkgs.coreutils}/bin/true";
              Slice = "app-amc.slice";
              MemoryMax = "2G";
            };
            systemd.user.slices.app-amc.Slice.MemoryMax = "3G";
          };
          canix-toolbelt.networking.directNetwork = {
            enable = true;
            package = pkgs.hello;
            systemServices.backup = "canix-background.slice";
            users.operator = {
              services.backend = "app-amc.slice";
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
  assert config.systemd.services.backup.serviceConfig.Slice == "canix-background-direct.slice";
  assert config.systemd.services.backup.serviceConfig.MemoryMax == "1G";
  assert config.systemd.slices.canix-background.sliceConfig.MemoryMax == "4G";
  assert home.systemd.user.services.backend.Service.Slice == "app-amc-direct.slice";
  assert home.systemd.user.services.backend.Service.MemoryMax == "2G";
  assert home.systemd.user.slices.app-amc.Slice.MemoryMax == "3G";
  assert builtins.elem "direct-network-agent-tools.service" home.systemd.user.services.backend.Unit.Requires;
  assert builtins.elem "user.slice/user-1000.slice/user@1000.service/app.slice/app-amc.slice/app-amc-direct.slice" policy.cgroupPaths;
  assert builtins.elem "user.slice/user-1000.slice/user@1000.service/agent.slice/agent-tools.slice" policy.cgroupPaths;
  assert !(builtins.elem "user.slice/user-1000.slice" policy.cgroupPaths);
  assert !(builtins.elem "user.slice/user-1000.slice/user@1000.service/app.slice" policy.cgroupPaths);
  assert policy.allowedUsers == [1000];
  assert config.system.nssDatabases.hosts == ["files" "mymachines" "myhostname" "dns"];
    pkgs.writeText "direct-network-eval" "Explicit cgroups retain all resource ancestors; desktop DNS and traffic remain separate.\n"
