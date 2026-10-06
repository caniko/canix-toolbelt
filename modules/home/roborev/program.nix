# Reusable Roborev Home Manager program module
# Provides declarative configuration and a wrapped executable with approved agent commands.
# This module has no osConfig dependency and does not manage enrollment or state migration.
{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit
    (lib)
    mkEnableOption
    mkIf
    mkMerge
    mkOption
    mkDefault
    types
    concatStringsSep
    escapeShellArg
    hasPrefix
    hasInfix
    hasSuffix
    ;

  cfg = config.programs.roborev;
  toml = pkgs.formats.toml {};
  commandKeys = {
    opencode = "opencode_cmd";
    codex = "codex_cmd";
    "claude-code" = "claude_code_cmd";
  };
  knownAgents = builtins.attrNames commandKeys;
  agentNames = builtins.attrNames cfg.agentCommands;
  reservedKeys = ["default_agent"] ++ builtins.attrValues commandKeys;
  reservedUsed = builtins.filter (key: builtins.hasAttr key cfg.settings) reservedKeys;

  absoluteString = value:
    builtins.isString value
    && hasPrefix "/" value
    && builtins.match ".*[[:cntrl:]].*" value == null;

  runtimeSecretPath = value:
    absoluteString value
    && value != "/nix/store"
    && !hasPrefix "/nix/store/" value
    && !hasInfix "//" value
    && !hasInfix "/../" "${value}/"
    && !hasInfix "/./" "${value}/";

  envReference = value:
    builtins.isString value
    && builtins.match "[$][{][A-Za-z_][A-Za-z0-9_]*[}]" value != null;

  appKey = lib.attrByPath ["ci" "github_app_private_key"] "" cfg.settings;
  anthropicKey = cfg.settings.anthropic_api_key or "";
  webTokenFile = lib.attrByPath ["web" "auth_token_file"] "" cfg.settings;
  embeddingKey = lib.attrByPath ["search" "embeddings" "api_key"] "" cfg.settings;

  # Ignore unknown names here so the assertion, rather than a missing-attribute
  # exception, reports mistakes in the interface.
  commandSettings = lib.mapAttrs' (
    name: path:
      lib.nameValuePair commandKeys.${name} path
  ) (lib.filterAttrs (name: _: builtins.hasAttr name commandKeys) cfg.agentCommands);

  effectiveSettings =
    cfg.settings
    // commandSettings
    // {
      default_agent = cfg.defaultAgent;
    };
  configFile = toml.generate "roborev-config.toml" effectiveSettings;
  agentBinDirs = map builtins.dirOf (builtins.attrValues cfg.agentCommands);
  basePackages = with pkgs; [
    bash
    coreutils
    findutils
    git
    gh
    gnugrep
    gnused
    openssh
    ripgrep
  ];
  runtimePath = concatStringsSep ":" (
    agentBinDirs ++ [(lib.makeBinPath (basePackages ++ cfg.extraPackages))]
  );
  telemetry =
    if cfg.enableTelemetry
    then "1"
    else "0";

  wrappedPackage = pkgs.symlinkJoin {
    name = "roborev-home-manager";
    paths = [cfg.package];
    nativeBuildInputs = [pkgs.makeWrapper];
    postBuild = ''
      wrapProgram "$out/bin/roborev" \
        --set ROBOREV_DATA_DIR ${escapeShellArg cfg.dataDir} \
        --set ROBOREV_TELEMETRY_ENABLED ${escapeShellArg telemetry} \
        --set PATH ${escapeShellArg runtimePath}
    '';
    meta.mainProgram = "roborev";
  };
in {
  options.programs.roborev = {
    _configFile = mkOption {
      type = types.package;
      internal = true;
      readOnly = true;
      description = "Generated configuration shared with the service.";
    };
    _runtimePath = mkOption {
      type = types.str;
      internal = true;
      readOnly = true;
      description = "Deterministic runtime command path.";
    };
    enable = mkEnableOption "roborev and its declarative configuration";

    package = mkOption {
      type = types.package;
      description = ''
        Explicit, pinned roborev package providing bin/roborev.
        No assumption is made that the caller's nixpkgs includes roborev.
      '';
    };

    finalPackage = mkOption {
      type = types.package;
      readOnly = true;
      description = "Wrapped package; use this executable for hooks and MCP clients.";
    };

    dataDir = mkOption {
      type = types.str;
      default = "${config.home.homeDirectory}/.roborev";
      description = ''
        Absolute data directory below the user's home. It contains both the
        managed config.toml and roborev-owned mutable state. No migration or
        deletion of existing state is performed.
      '';
    };

    defaultAgent = mkOption {
      type = types.enum knownAgents;
      default = "opencode";
      description = "Explicit default harness; it must exist in agentCommands.";
    };

    agentCommands = mkOption {
      type = types.attrsOf types.str;
      default = {};
      description = ''
        Mapping of opencode, codex, or claude-code to absolute executable paths.
        Values are single executable paths, not shell commands with arguments.
        Pass an existing approved containment wrapper when one is in use.
        These are execution choices, not a security allowlist.
      '';
    };

    settings = mkOption {
      inherit (toml) type;
      default = {};
      description = ''
        Non-secret upstream TOML settings. default_agent and *_cmd for the
        supported harnesses are owned by defaultAgent and agentCommands.
        Use runtime file paths or upstream-supported environment references
        for credentials. Never put plaintext secrets in this attribute set.
      '';
    };

    enableTelemetry = mkOption {
      type = types.bool;
      default = false;
      description = "Enable roborev telemetry (does not change agent telemetry).";
    };

    extraPackages = mkOption {
      type = types.listOf types.package;
      default = [];
      description = ''
        Additional runtime command dependencies. Git, gh, SSH, Bash and basic
        inspection tools are always included. The daemon gets their PATH plus
        agent executable directories, without interactive shell setup. Add
        project tooling such as nix explicitly when required.
      '';
    };
  };

  config = mkMerge [
    (mkIf cfg.enable {
      assertions = [
        {
          assertion =
            absoluteString cfg.dataDir
            && hasPrefix "${config.home.homeDirectory}/" cfg.dataDir
            && !hasInfix "/../" "${cfg.dataDir}/"
            && !hasInfix "/./" "${cfg.dataDir}/"
            && !hasInfix "//" cfg.dataDir
            && !hasSuffix "/" cfg.dataDir;
          message = "programs.roborev.dataDir must be a normalized absolute directory below homeDirectory.";
        }
        {
          assertion = builtins.all (name: builtins.elem name knownAgents) agentNames;
          message = "programs.roborev.agentCommands accepts only opencode, codex and claude-code.";
        }
        {
          assertion = builtins.all absoluteString (builtins.attrValues cfg.agentCommands);
          message = "roborev agentCommands must be absolute executable paths, without newline characters.";
        }
        {
          assertion = builtins.all (path: !hasInfix ":" (builtins.dirOf path)) (builtins.attrValues cfg.agentCommands);
          message = "roborev agent command directories must not contain PATH separators.";
        }
        {
          assertion = builtins.hasAttr cfg.defaultAgent cfg.agentCommands;
          message = "The roborev defaultAgent must have an explicit agentCommands entry.";
        }
        {
          assertion = reservedUsed == [];
          message = "Use roborev.defaultAgent and agentCommands instead of these settings: ${concatStringsSep ", " reservedUsed}";
        }
        {
          assertion = (cfg.settings.auth_key or "") == "";
          message = "This module does not materialize a secret daemon auth_key. Use the private Unix socket, or add tested runtime secret support; never put a key in settings.";
        }
        {
          assertion = (lib.attrByPath ["web" "auth_token"] "" cfg.settings) == "";
          message = "Use web.auth_token_file with a runtime path, not an inline web.auth_token.";
        }
        {
          assertion = anthropicKey == "" || envReference anthropicKey;
          message = "anthropic_api_key must be absent, empty, or an upstream environment-variable reference, never a literal key.";
        }
        {
          assertion = webTokenFile == "" || runtimeSecretPath webTokenFile;
          message = "web.auth_token_file must be an absolute runtime string outside the Nix store.";
        }
        {
          assertion = embeddingKey == "";
          message = "Embedding credentials are outside this module's tested baseline; never serialize them in settings.";
        }
        {
          assertion =
            builtins.isString appKey
            && (
              appKey == "" || envReference appKey || runtimeSecretPath appKey
            );
          message = "ci.github_app_private_key must be a runtime file path or environment reference, never inline PEM.";
        }
      ];

      programs.roborev.finalPackage = wrappedPackage;
      programs.roborev._configFile = configFile;
      programs.roborev._runtimePath = runtimePath;
      programs.roborev.settings = {
        server_addr = mkDefault "unix://";
        max_workers = mkDefault 2;
        isolate_reviews = mkDefault true;
        allow_unsafe_agents = mkDefault false;
        disable_codex_sandbox = mkDefault false;
        # Preserve the explicit local harness's user configuration policy.
        agent.codex.ignore_review_user_config = mkDefault false;
        ci.enabled = mkDefault false;
        web.enabled = mkDefault false;
        mcp.enabled = mkDefault false;
        sync.enabled = mkDefault false;
        budget.enabled = mkDefault false;
        auto_design_review.enabled = mkDefault false;
        auto_design_review.hook_enabled = mkDefault false;
        advanced.tasks_enabled = mkDefault false;
      };

      home.packages = [cfg.finalPackage];
      home.file."roborev-config" = {
        target = "${cfg.dataDir}/config.toml";
        source = configFile;
        # Do not force an overwrite of an existing unmanaged configuration.
      };
      home.activation.roborevDataDirCheck = lib.hm.dag.entryBefore ["writeBoundary"] ''
        roborev_dir=${escapeShellArg cfg.dataDir}
        roborev_home=${escapeShellArg config.home.homeDirectory}
        roborev_uid=$(${pkgs.coreutils}/bin/id -u)
        roborev_current="$roborev_dir"
        while true; do
          if [[ -L "$roborev_current" || ( -e "$roborev_current" && ! -d "$roborev_current" ) ]]; then
            echo "roborev: data directory or ancestor is a symlink/non-directory: $roborev_current" >&2
            exit 1
          fi
          if [[ -d "$roborev_current" && "$(${pkgs.coreutils}/bin/stat -c %u "$roborev_current")" != "$roborev_uid" ]]; then
            echo "roborev: data directory or home ancestor has another owner" >&2
            exit 1
          fi
          [[ "$roborev_current" == "$roborev_home" ]] && break
          roborev_current="$(${pkgs.coreutils}/bin/dirname "$roborev_current")"
        done
        roborev_config="$roborev_dir/config.toml"
        if [[ -e "$roborev_config" || -L "$roborev_config" ]]; then
          roborev_old="''${oldGenPath:-}/home-files/"${escapeShellArg "${lib.removePrefix "${config.home.homeDirectory}/" cfg.dataDir}/config.toml"}
          if [[ ! -L "$roborev_config" || ! -e "$roborev_old" || "$(${pkgs.coreutils}/bin/readlink -f "$roborev_config")" != "$(${pkgs.coreutils}/bin/readlink -f "$roborev_old")" ]]; then
            echo "roborev: refusing unmanaged config.toml collision; preserve it before enabling the module" >&2
            exit 1
          fi
        fi
      '';
      home.activation.roborevDataDir =
        lib.hm.dag.entryBetween
        ["linkGeneration"] ["writeBoundary"] ''
          run ${pkgs.coreutils}/bin/install -d -m 0700 ${escapeShellArg cfg.dataDir}
        '';
    })
  ];
}
