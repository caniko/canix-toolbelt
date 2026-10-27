{pkgs}: let
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  inherit (pkgs) lib;

  profile = host: system:
    (lib.evalModules {
      modules = [
        ../modules/nixos/registry/host-profile.nix
        {
          options.networking.hostName = lib.mkOption {type = lib.types.str;};
          options.nixpkgs.hostPlatform.system = lib.mkOption {type = lib.types.str;};
          config = {
            networking.hostName = host;
            nixpkgs.hostPlatform.system = system;
            canix-toolbelt.hosts = {
              hub = {
                deviceType = "server";
                flakeRoot = "/srv/flake";
              };
              desk = {
                deviceType = "desktop";
                dataRoot = "/data";
                projectStateRoot = "/data/state";
              };
            };
          };
        }
      ];
    })
    .config
    .canix-toolbelt
    .hostProfile;
  hub = profile "hub" "aarch64-linux";
  desk = profile "desk" "x86_64-linux";
in
  mkEvalCheck {
    name = "host-profile-eval";
    assertions = [
      {
        name = "server-facts";
        assertion = hub.deviceType == "server" && hub.isServer && hub.isHeadless && hub.isAarch64 && hub.flakeRoot == "/srv/flake" && hub.dataRoot == null;
        message = "server host facts must derive from its registry entry";
      }
      {
        name = "desktop-facts";
        assertion = !desk.isServer && !desk.isHeadless && !desk.isAarch64 && desk.dataRoot == "/data" && desk.projectStateRoot == "/data/state" && desk.flakeRoot == null;
        message = "desktop host facts must derive from its registry entry";
      }
      {
        name = "unknown-host";
        assertion = !(builtins.tryEval (profile "ghost" "x86_64-linux").deviceType).success;
        message = "an unregistered host must fail evaluation";
      }
    ];
  }
