{
  inputs,
  pkgs,
}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  dummyPkg = pkgs.emptyDirectory;
  hasPackage = cfg: lib.any (p: p.outPath == dummyPkg.outPath) cfg.home.packages;

  evaluate = settings:
    (inputs.home-manager.lib.homeManagerConfiguration {
      inherit pkgs;
      modules = [
        ../modules/home/rustdesk-client.nix
        {
          home = {
            username = "fixture";
            homeDirectory = "/home/fixture";
            stateVersion = "25.05";
          };
          canix-toolbelt.rustdesk-client = settings;
        }
      ];
    }).config;

  disabled = evaluate {
    package = dummyPkg;
  };
  noPassword = evaluate {
    enable = true;
    package = dummyPkg;
    rendezvousServer = "10.123.0.1";
  };
  withPassword = evaluate {
    enable = true;
    package = dummyPkg;
    rendezvousServer = "10.123.0.1";
    apiPort = 22114;
    usePassword = true;
    passwordFile = "/run/secrets/rustdesk-password";
  };
  passwordService = withPassword.systemd.user.services.rustdesk-set-password;
in
  mkEvalCheck {
    name = "rustdesk-client-eval";
    resultMessage = "rustdesk-client Home Manager module evaluation passed";
    assertions = [
      {
        name = "disabled-clean";
        assertion =
          !(hasPackage disabled)
          && !(disabled.home.activation ? rustdesk-preset)
          && !(disabled.systemd.user.services ? rustdesk-set-password);
        message = "disabled rustdesk-client must not add packages, activation scripts, or systemd units";
      }
      {
        name = "enabled-without-password";
        assertion =
          hasPackage noPassword
          && (noPassword.home.activation ? rustdesk-preset)
          && !(noPassword.systemd.user.services ? rustdesk-set-password);
        message = "enabled rustdesk-client without password must install package and activation preset without systemd service";
      }
      {
        name = "enabled-with-password";
        assertion =
          hasPackage withPassword
          && (withPassword.home.activation ? rustdesk-preset)
          && (withPassword.systemd.user.services ? rustdesk-set-password)
          && (lib.toList passwordService.Service.ExecStart != [])
          && lib.hasSuffix "-rustdesk-set-password" (builtins.head (lib.toList passwordService.Service.ExecStart))
          && passwordService.Service.RemainAfterExit == true;
        message = "enabled rustdesk-client with usePassword and passwordFile must define rustdesk-set-password service with ExecStart and RemainAfterExit";
      }
    ];
  }
