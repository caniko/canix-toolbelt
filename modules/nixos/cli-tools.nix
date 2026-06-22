{config, lib, pkgs, ...}: let
  inherit (lib) mkEnableOption mkIf mkOption types;

  cfg = config.canix-toolbelt.packages.cliTools;

  knownTools = {
    jq = {
      description = "JSON processor";
      package = pkgs.jq;
    };
    bat = {
      description = "Enhanced cat with syntax highlighting";
      package = pkgs.bat;
    };
    ripgrep = {
      description = "Fast grep replacement";
      package = pkgs.ripgrep;
    };
    eza = {
      description = "Modern ls replacement";
      package = pkgs.eza;
    };
    fd = {
      description = "Fast file search";
      package = pkgs.fd;
    };
    fzf = {
      description = "Fuzzy finder";
      package = pkgs.fzf;
    };
    htop = {
      description = "Interactive process viewer";
      package = pkgs.htop;
    };
    yq = {
      description = "YAML/JSON processor";
      package = pkgs.yq;
    };
    tree = {
      description = "Directory tree view";
      package = pkgs.tree;
    };
    unzip = {
      description = "Zip extraction utility";
      package = pkgs.unzip;
    };
    btop = {
      description = "Resource monitor";
      package = pkgs.btop;
    };
    fastfetch = {
      description = "System information tool";
      package = pkgs.fastfetch;
    };
    tmux = {
      description = "Terminal multiplexer";
      package = pkgs.tmux;
    };
    dust = {
      description = "Disk usage analyzer";
      package = pkgs.dust;
    };
  };

  toolOptions = builtins.mapAttrs (name: entry: {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Whether to install ${entry.description} system-wide.";
    };
    package = mkOption {
      type = types.package;
      default = entry.package;
      defaultText = lib.literalExpression "pkgs.${name}";
      description = "Package to use for ${entry.description}.";
    };
  }) knownTools;

  enabledTools = lib.filterAttrs (n: v: n != "enable" && v.enable) cfg;
in {
  options.canix-toolbelt.packages.cliTools = {
    enable = mkEnableOption "system-level CLI tools";
  } // toolOptions;

  config = mkIf cfg.enable {
    environment.systemPackages = builtins.attrValues (
      builtins.mapAttrs (_: t: t.package) enabledTools
    );
  };
}
