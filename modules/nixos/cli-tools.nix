{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mkEnableOption mkIf mkOption types;
  safety = import ../../lib/agent-safety.nix {inherit lib;};

  cfg = config.canix-toolbelt.packages.cliTools;

  builtInKeys = ["enable" "opencodeRules" "agentSafety"];

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
    zellij = {
      description = "Terminal multiplexer";
      package = pkgs.zellij;
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

  toolOptions =
    builtins.mapAttrs (name: entry: {
      enable = mkOption {
        type = types.bool;
        default = false;
        description = "Whether to install ${entry.description} system-wide.";
      };
      opencodePermission = mkOption {
        type = types.nullOr (types.enum ["allow" "deny" "ask"]);
        default = null;
        description = ''
          Compatibility action for the removed one-rule OpenCode integration.
          Use autoSafe for command-aware declarations.
        '';
      };
      autoSafe = safety.mkAutoSafeOption "Prompt-safe command declaration for ${entry.description}.";
      package = mkOption {
        type = types.package;
        default = entry.package;
        defaultText = lib.literalExpression "pkgs.${name}";
        description = "Package to use for ${entry.description}.";
      };
    })
    knownTools;

  enabledTools = lib.filterAttrs (n: v: !(builtins.elem n builtInKeys) && v.enable) cfg;
  opencodeTools = lib.filterAttrs (n: v: !(builtins.elem n builtInKeys) && (v.autoSafe != null || v.opencodePermission != null)) cfg;
  autoSafeToolRules = builtins.foldl' (
    acc: name: let
      tool = opencodeTools.${name};
      entry = builtins.getAttr name knownTools;
      bins = entry.binPatterns or [name];
      autoSafe =
        if tool.autoSafe == null
        then null
        else
          tool.autoSafe
          // {
            executables =
              if tool.autoSafe.executables == []
              then bins
              else tool.autoSafe.executables;
          };
      rendered =
        if autoSafe == null
        then {}
        else
          safety.renderPrograms {
            ${name} = {inherit autoSafe;};
          };
      compatibility =
        if tool.opencodePermission == null
        then {}
        else
          builtins.listToAttrs (map (bin: {
              name = "${bin} *";
              value = tool.opencodePermission;
            })
            bins);
    in
      acc // rendered // compatibility
  ) {} (builtins.attrNames opencodeTools);
in {
  options.canix-toolbelt.packages.cliTools =
    {
      enable = mkEnableOption "system-level CLI tools";
      opencodeRules = mkOption {
        type = types.attrsOf types.str;
        readOnly = true;
        internal = true;
        description = "Bash permission rules for opencode derived from enabled CLI tools.";
      };
      agentSafety = mkOption {
        type = types.attrsOf types.attrs;
        readOnly = true;
        internal = true;
        description = "Normalized agent-safety declarations for integrated Home Manager.";
      };
    }
    // toolOptions;

  config = mkIf cfg.enable {
    environment.systemPackages = builtins.attrValues (
      builtins.mapAttrs (_: t: t.package) enabledTools
    );

    canix-toolbelt.packages.cliTools.opencodeRules = autoSafeToolRules;
    canix-toolbelt.packages.cliTools.agentSafety =
      builtins.mapAttrs (name: tool: let
        bins = (builtins.getAttr name knownTools).binPatterns or [name];
      in {
        autoSafe =
          if tool.autoSafe == null
          then null
          else
            tool.autoSafe
            // {
              executables =
                if tool.autoSafe.executables == []
                then bins
                else tool.autoSafe.executables;
            };
        executables = bins;
      })
      opencodeTools;
  };
}
