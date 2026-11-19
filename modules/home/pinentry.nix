{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.pinentry;
  pinentry = import ../../lib/pinentry.nix {inherit lib;};
  packages = pinentry.mkPackages {
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
    zellij = lib.mkOption {
      type = lib.types.package;
      default = pinentry.mkZellij {inherit pkgs;};
      defaultText = lib.literalExpression "lib.pinentry.mkZellij { inherit pkgs; }";
      description = "Zellij used by pinentry clients; select the same package for session servers.";
    };
    gnupg = lib.mkPackageOption pkgs "gnupg" {};
    rage = lib.mkPackageOption pkgs "rage" {};
    tty = lib.mkPackageOption pkgs "pinentry-tty" {};
    qt = lib.mkPackageOption pkgs "pinentry-qt" {};
    peers = lib.mkOption {
      type = lib.types.listOf (lib.types.nonEmptyListOf (lib.types.strMatching "[^\t\n]+"));
      default = [];
      example = [["ssh" "-o" "BatchMode=yes" "-o" "ConnectTimeout=3" "-S" "none" "me@laptop" "--" "canix-toolbelt-pinentry"]];
      description = ''
        Commands that reach another host's router, tried in order; `--serve`
        is appended. A desktop or headless request whose local seat is locked
        or absent prompts on the first peer whose graphical seat is active and
        unlocked (an attached Zellij session there, else Qt). The PIN returns
        over the command's stdout to this host's agent and card.
      '';
    };
    packages = lib.mkOption {
      type = lib.types.attrsOf lib.types.package;
      readOnly = true;
      description = "Router and request-aware GPG/rage packages for explicit consumer wiring.";
    };
  };

  config = lib.mkMerge [
    {
      canix-toolbelt.pinentry.packages =
        if cfg.enable
        then packages
        else {};
    }
    (lib.mkIf cfg.enable {
      # Peers run `canix-toolbelt-pinentry --serve` from the login PATH. Only
      # that name: the router's generic `pinentry` alias stays scoped to rage.
      home.packages = [
        (pkgs.runCommand "canix-toolbelt-pinentry-serve" {} ''
          mkdir -p "$out/bin"
          ln -s ${lib.getExe packages.router} "$out/bin/canix-toolbelt-pinentry"
        '')
      ];
      xdg.configFile."canix-toolbelt/pinentry-peers" = lib.mkIf (cfg.peers != []) {
        text = lib.concatMapStrings (peer: lib.concatStringsSep "\t" peer + "\n") cfg.peers;
      };
    })
    (lib.mkIf (cfg.enable && cfg.gpgIntegration) {
      services.gpg-agent.enable = lib.mkDefault true;
      services.gpg-agent.pinentry = {
        package = packages.router;
        program = "canix-toolbelt-pinentry-agent";
      };
      programs.gpg = {
        enable = true;
        package = lib.hiPrio packages.gpg;
      };
      programs.nushell.extraConfig = lib.mkIf (config.programs.nushell.enable && config.services.gpg-agent.enableNushellIntegration) (lib.mkAfter ''
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
    (lib.mkIf (cfg.enable && cfg.rageIntegration) {
      home.packages = [(lib.hiPrio packages.rage)];
    })
  ];
}
