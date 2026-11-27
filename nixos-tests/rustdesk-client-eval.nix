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
  usePasswordWithoutFile = evaluate {
    enable = true;
    package = dummyPkg;
    rendezvousServer = "10.123.0.1";
    usePassword = true;
    passwordFile = null;
  };
  passwordFileWithoutFlag = evaluate {
    enable = true;
    package = dummyPkg;
    rendezvousServer = "10.123.0.1";
    usePassword = false;
    passwordFile = "/run/secrets/rustdesk-password";
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
  passwordExecStart = builtins.head (lib.toList passwordService.Service.ExecStart);
in
  mkEvalCheck {
    name = "rustdesk-client-eval";
    resultMessage = "rustdesk-client Home Manager module evaluation passed";
    runtimeScript = ''
      grep -Fq 'http://10.123.0.1:22114/api/devices/self/access-policy' "${passwordExecStart}"
      grep -Fq 'password_file="/run/secrets/rustdesk-password"' "${passwordExecStart}"
      grep -Fq -- '--rawfile password "$password_file"' "${passwordExecStart}"
      grep -Fq -- '--data-binary @-' "${passwordExecStart}"
      grep -Fq -- 'curl -sS' "${passwordExecStart}"
    '';
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
          && noPassword.home.activation.rustdesk-preset.after == ["writeBoundary"]
          && lib.hasInfix "RustDesk2.toml" noPassword.home.activation.rustdesk-preset.data
          && !(noPassword.systemd.user.services ? rustdesk-set-password);
        message = "enabled rustdesk-client without password must install package and activation preset without systemd service";
      }
      {
        name = "partial-password-options-omit-service";
        assertion =
          !(usePasswordWithoutFile.systemd.user.services ? rustdesk-set-password)
          && !(passwordFileWithoutFlag.systemd.user.services ? rustdesk-set-password);
        message = "rustdesk-set-password must only be installed when both usePassword and passwordFile are set";
      }
      {
        name = "enabled-with-password";
        assertion =
          hasPackage withPassword
          && (withPassword.home.activation ? rustdesk-preset)
          && (withPassword.systemd.user.services ? rustdesk-set-password)
          && (lib.toList passwordService.Service.ExecStart != [])
          && lib.hasSuffix "-rustdesk-set-password" passwordExecStart
          && passwordService.Service.RemainAfterExit == true
          && passwordService.Unit.After == ["graphical-session.target"]
          && passwordService.Unit.BindsTo == ["graphical-session.target"]
          && passwordService.Install.WantedBy == ["default.target"];
        message = "enabled rustdesk-client with usePassword and passwordFile must define rustdesk-set-password service with expected Unit, Service, and Install settings";
      }
    ];
  }
