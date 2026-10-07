{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.pinentry;
  packages = (import ../../lib/pinentry.nix {inherit lib;}).mkPackages {
    inherit pkgs;
    inherit (cfg) zellij gnupg rage tty qt;
  };
  context = "${lib.getExe packages.router} --context";
  posixContext = ''export PINENTRY_USER_DATA="$(${context})"'';
in {
  options.canix-toolbelt.pinentry = {
    enable = lib.mkEnableOption "request-local Zellij/Qt/TTY pinentry";
    gpgIntegration = lib.mkEnableOption "GPG request-context forwarding";
    rageIntegration = lib.mkEnableOption "age plugin PIN and confirmation routing through rage";
    zellij = lib.mkPackageOption pkgs "zellij" {};
    gnupg = lib.mkPackageOption pkgs "gnupg" {};
    rage = lib.mkPackageOption pkgs "rage" {};
    tty = lib.mkPackageOption pkgs "pinentry-tty" {};
    qt = lib.mkPackageOption pkgs "pinentry-qt" {};
    packages = lib.mkOption {
      type = lib.types.attrsOf lib.types.package;
      readOnly = true;
      default = {};
      description = "Router and request-aware GPG/rage packages for explicit consumer wiring.";
    };
  };

  config = lib.mkIf cfg.enable (lib.mkMerge [
    {canix-toolbelt.pinentry.packages = packages;}
    (lib.mkIf cfg.gpgIntegration {
      services.gpg-agent.enable = lib.mkDefault true;
      services.gpg-agent.pinentry = {
        package = packages.router;
        program = "canix-toolbelt-pinentry-agent";
      };
      programs.gpg = {
        enable = true;
        package = packages.gpg;
      };
      programs.nushell.extraConfig = lib.mkIf config.programs.nushell.enable (lib.mkAfter ''
        let pinentry_context = {||
          $env.PINENTRY_USER_DATA = (^${context} | str trim)
        }
        do --env $pinentry_context
        $env.config.hooks.pre_execution = (
          $env.config.hooks.pre_execution | append $pinentry_context
        )
      '');
      programs.bash.initExtra = lib.mkIf config.services.gpg-agent.enableBashIntegration posixContext;
      programs.zsh.initContent = lib.mkIf config.services.gpg-agent.enableZshIntegration posixContext;
      programs.fish.interactiveShellInit = lib.mkIf config.services.gpg-agent.enableFishIntegration ''
        set -gx PINENTRY_USER_DATA (${context})
      '';
      home.file."${config.programs.gpg.homedir}/gpg-agent.conf".onChange = ''
        ${packages.gpg}/bin/gpgconf --homedir ${lib.escapeShellArg config.programs.gpg.homedir} --reload gpg-agent
      '';
    })
    (lib.mkIf cfg.rageIntegration {
      home.packages = [packages.rage];
    })
  ]);
}
