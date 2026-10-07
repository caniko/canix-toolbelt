{
  inputs,
  pkgs,
}: let
  inherit (pkgs) lib;
  evaluate = {
    settings ? {},
    nushell ? false,
    nushellIntegration ? true,
  }:
    (inputs.home-manager.lib.homeManagerConfiguration {
      inherit pkgs;
      modules = [
        ../modules/home/pinentry.nix
        {
          home = {
            username = "fixture";
            homeDirectory = "/home/fixture";
            stateVersion = "25.05";
          };
          canix-toolbelt.pinentry = settings;
          programs.nushell.enable = nushell;
          services.gpg-agent.enableNushellIntegration = nushellIntegration;
        }
      ];
    }).config;
  disabled = evaluate {};
  direct = evaluate {
    settings = {
      enable = true;
      rageIntegration = true;
    };
  };
  gpg = evaluate {
    settings = {
      enable = true;
      gpgIntegration = true;
    };
  };
  nushell = evaluate {
    settings = {
      enable = true;
      gpgIntegration = true;
    };
    nushell = true;
  };
  nushellOptOut = evaluate {
    settings = {
      enable = true;
      gpgIntegration = true;
    };
    nushell = true;
    nushellIntegration = false;
  };
in
  assert !disabled.programs.gpg.enable;
  assert disabled.canix-toolbelt.pinentry.packages == {};
  assert !direct.programs.gpg.enable;
  assert lib.elem direct.canix-toolbelt.pinentry.packages.rage direct.home.packages;
  assert gpg.programs.gpg.package == gpg.canix-toolbelt.pinentry.packages.gpg;
  assert gpg.services.gpg-agent.pinentry.program == "canix-toolbelt-pinentry-agent";
  assert !gpg.programs.nushell.enable;
  # Home Manager may add its own GPG_TTY hook; disabled Nushell must not
  # receive the router's request-context hook.
  assert !lib.hasInfix "PINENTRY_USER_DATA" gpg.programs.nushell.extraConfig;
  assert lib.hasInfix "PINENTRY_USER_DATA" nushell.programs.nushell.extraConfig;
  assert nushellOptOut.programs.nushell.extraConfig == "";
    pkgs.writeText "pinentry-home-eval" "ok"
