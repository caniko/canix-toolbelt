{
  inputs,
  pkgs,
}: let
  inherit (pkgs) lib;
  pinentry = import ../lib/pinentry.nix {inherit lib;};
  evaluate = {
    settings ? {},
    nushell ? false,
    nushellIntegration ? true,
    rawRage ? false,
    rawGnupg ? false,
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
          home.packages = lib.optional rawRage pkgs.rage ++ lib.optional rawGnupg pkgs.gnupg;
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
  explicitZellij = evaluate {
    settings = {
      enable = true;
      inherit (pkgs) zellij;
    };
  };
  coexist = evaluate {
    settings = {
      enable = true;
      rageIntegration = true;
      gpgIntegration = true;
    };
    rawRage = true;
    rawGnupg = true;
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
  remote = evaluate {
    settings = {
      enable = true;
      peers = [["ssh" "me@peer" "--" "canix-toolbelt-pinentry"]];
    };
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
  assert direct.canix-toolbelt.pinentry.zellij.drvPath == (pinentry.mkZellij {inherit pkgs;}).drvPath;
  assert direct.canix-toolbelt.pinentry.packages.zellij.drvPath == direct.canix-toolbelt.pinentry.zellij.drvPath;
  assert explicitZellij.canix-toolbelt.pinentry.packages.zellij.drvPath == pkgs.zellij.drvPath;
  assert lib.any (package: package.outPath == direct.canix-toolbelt.pinentry.packages.rage.outPath) direct.home.packages;
  assert gpg.programs.gpg.package.outPath == gpg.canix-toolbelt.pinentry.packages.gpg.outPath;
  assert gpg.services.gpg-agent.pinentry.program == "canix-toolbelt-pinentry-agent";
  assert !gpg.programs.nushell.enable;
  # Home Manager may add its own GPG_TTY hook; disabled Nushell must not
  # receive the router's request-context hook.
  assert !lib.hasInfix "PINENTRY_USER_DATA" gpg.programs.nushell.extraConfig;
  assert lib.hasInfix "PINENTRY_USER_DATA" nushell.programs.nushell.extraConfig;
  assert nushellOptOut.programs.nushell.extraConfig == "";
  assert remote.xdg.configFile."canix-toolbelt/pinentry-peers".text == "ssh\tme@peer\t--\tcanix-toolbelt-pinentry\n";
  assert !(direct.xdg.configFile ? "canix-toolbelt/pinentry-peers");
  # Peers invoke the router by name from the login PATH.
  assert lib.any (package: package.outPath == remote.canix-toolbelt.pinentry.packages.router.outPath) remote.home.packages;
    pkgs.runCommand "pinentry-home-eval" {} ''
      test "$(readlink -f ${coexist.home.path}/bin/rage)" = "${coexist.canix-toolbelt.pinentry.packages.rage}/bin/rage"
      for program in gpg gpg2; do
        test "$(readlink -f ${coexist.home.path}/bin/$program)" = "${lib.getExe coexist.canix-toolbelt.pinentry.packages.router}"
      done
      touch "$out"
    ''
