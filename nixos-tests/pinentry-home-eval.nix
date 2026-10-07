{
  inputs,
  pkgs,
}: let
  inherit (pkgs) lib;
  evaluate = settings:
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
        }
      ];
    }).config;
  disabled = evaluate {};
  direct = evaluate {
    enable = true;
    rageIntegration = true;
  };
  gpg = evaluate {
    enable = true;
    gpgIntegration = true;
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
    pkgs.writeText "pinentry-home-eval" "ok"
