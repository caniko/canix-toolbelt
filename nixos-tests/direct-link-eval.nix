{pkgs}: let
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  evaluate = travel:
    (import "${pkgs.path}/nixos/lib/eval-config.nix" {
      system = "x86_64-linux";
      modules = [
        ({lib, ...}: {
          options.home-manager.sharedModules = lib.mkOption {
            type = lib.types.listOf lib.types.deferredModule;
            default = [];
          };
        })
        ../modules/nixos/networking/direct-link.nix
        ../modules/nixos/profiles.nix
        {
          networking.hostName = "client";
          system.stateVersion = "25.11";

          canix-toolbelt = {
            hosts = {
              client = {
                deviceType = "laptop";
                directLinkIp = "10.10.0.2";
                directLinkMac = "00:11:22:33:44:55";
              };
              gateway = {
                deviceType = "desktop";
                directLinkIp = "10.10.0.1";
              };
            };
            networking.directLink = {
              enable = true;
              role = "client";
              gateway = "gateway";
            };
            profiles.travel.enable = travel;
          };
        }
      ];
    }).config.networking.networkmanager.ensureProfiles.profiles.direct-link;

  normal = evaluate false;
  travel = evaluate true;
in
  mkEvalCheck {
    name = "direct-link-eval";
    resultMessage = "direct-link client profile is independent of travel mode";
    assertions = [
      {
        name = "normal-profile";
        assertion = normal.ipv4.addresses == "10.10.0.2/24";
        message = "direct-link profile must exist in normal mode";
      }
      {
        name = "travel-profile";
        assertion = travel == normal;
        message = "travel mode must not suppress or alter the direct-link profile";
      }
      {
        name = "mac-bound";
        assertion = travel.ethernet.mac-address == "00:11:22:33:44:55";
        message = "direct-link profile must remain bound to the declared adapter";
      }
    ];
  }
