{config, lib, pkgs, ...}: let
  inherit (lib) mkEnableOption mkIf mkOption types;

  cfg = config.canix-toolbelt.packages.cliTools;

  builtInKeys = ["enable" "opencodeRules"];

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
      binPatterns = ["rg"];
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

    choose = {
      description = "Modern cut replacement";
      package = pkgs.choose;
    };
    csvlens = {
      description = "CSV/TSV viewer";
      package = pkgs.csvlens;
    };
    delta = {
      description = "Syntax-highlighted diff viewer";
      package = pkgs.delta;
    };
    doggo = {
      description = "Modern DNS lookup tool";
      package = pkgs.doggo;
    };
    duf = {
      description = "Modern df replacement";
      package = pkgs.duf;
    };
    gdu = {
      description = "Fast disk usage analyzer";
      package = pkgs.gdu;
    };
    hexyl = {
      description = "Modern hex viewer";
      package = pkgs.hexyl;
    };
    huniq = {
      description = "Modern dedup tool";
      package = pkgs.huniq;
    };
    hyperfine = {
      description = "Command benchmarking tool";
      package = pkgs.hyperfine;
    };
    just = {
      description = "Modern command runner";
      package = pkgs.just;
    };
    miller = {
      description = "Structured data processor (CSV/JSON)";
      package = pkgs.miller;
      binPatterns = ["mlr"];
    };
    ouch = {
      description = "Universal archive tool";
      package = pkgs.ouch;
    };
    procs = {
      description = "Modern ps replacement";
      package = pkgs.procs;
    };
    sd = {
      description = "Modern sed replacement";
      package = pkgs.sd;
    };
    trash-cli = {
      description = "Trash can CLI";
      package = pkgs.trash-cli;
      binPatterns = ["trash-put" "trash-list" "trash-restore" "trash-empty" "trash-rm"];
    };
    websocat = {
      description = "WebSocket client";
      package = pkgs.websocat;
    };
    xh = {
      description = "Modern HTTP client";
      package = pkgs.xh;
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

  enabledTools = lib.filterAttrs (n: v: !(builtins.elem n builtInKeys) && v.enable) cfg;
in {
  options.canix-toolbelt.packages.cliTools = {
    enable = mkEnableOption "system-level CLI tools";
    opencodeRules = mkOption {
      type = types.attrsOf types.str;
      readOnly = true;
      internal = true;
      description = "Bash permission rules for opencode derived from enabled CLI tools.";
    };
  } // toolOptions;

  config = mkIf cfg.enable {
    environment.systemPackages = builtins.attrValues (
      builtins.mapAttrs (_: t: t.package) enabledTools
    );

    canix-toolbelt.packages.cliTools.opencodeRules =
      builtins.foldl' (acc: name:
        let entry = builtins.getAttr name knownTools;
            bins = entry.binPatterns or [name];
        in acc // builtins.listToAttrs (map (bin: {
          name = "${bin} *";
          value = "allow";
        }) bins)
      ) {} (builtins.attrNames enabledTools);
  };
}
