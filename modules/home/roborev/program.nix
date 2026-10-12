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
  schema = import ./schema.nix {inherit lib toml;};
  settings = schema.clean cfg.settings;
  commandKeys = {
    opencode = "opencode_cmd";
    codex = "codex_cmd";
    "claude-code" = "claude_code_cmd";
    gemini = "gemini_cmd";
    cursor = "cursor_cmd";
    pi = "pi_cmd";
    grok = "grok_cmd";
  };
  commandNames = {
    copilot = "copilot";
    droid = "droid";
    kilo = "kilo";
    kiro = "kiro-cli";
  };
  knownAgents = builtins.attrNames (commandKeys // commandNames);
  agentNames = builtins.attrNames cfg.agentCommands;
  commandConflicts =
    builtins.filter (
      name:
        builtins.hasAttr name commandKeys
        && builtins.hasAttr commandKeys.${name} settings
        && settings.${commandKeys.${name}} != cfg.agentCommands.${name}
    )
    agentNames;
  acpName = lib.removePrefix "acp." cfg.defaultAgent;
  acpCommand = lib.attrByPath ["acp" acpName "command"] "" settings;
  configuredCommand =
    if builtins.hasAttr cfg.defaultAgent commandKeys
    then settings.${commandKeys.${cfg.defaultAgent}} or ""
    else "";

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
    && !hasInfix "/./" "${value}/"
    && !hasSuffix "/" value;

  envReference = value:
    builtins.isString value
    && builtins.match "[$][{][A-Za-z_][A-Za-z0-9_]*[}]" value != null;

  # Ignore unknown names here so the assertion, rather than a missing-attribute
  # exception, reports mistakes in the interface.
  commandSettings = lib.mapAttrs' (
    name: path:
      lib.nameValuePair commandKeys.${name} path
  ) (lib.filterAttrs (name: _: builtins.hasAttr name commandKeys) cfg.agentCommands);

  effectiveSettings =
    settings
    // commandSettings
    // {
      default_agent = cfg.defaultAgent;
    };
  configFile =
    if cfg.runtimeConfigFile == null
    then toml.generate "roborev-config.toml" effectiveSettings
    else config.lib.file.mkOutOfStoreSymlink cfg.runtimeConfigFile;
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
  baseRuntimePath = concatStringsSep ":" (
    agentBinDirs ++ [(lib.makeBinPath (basePackages ++ cfg.extraPackages))]
  );
  runtimePath = "${wrappedPackage}/libexec/roborev-agents:${baseRuntimePath}";
  environment =
    cfg.environment
    // lib.optionalAttrs (cfg.colorMode != null) {
      ROBOREV_COLOR_MODE = cfg.colorMode;
    };
  environmentFlags = lib.concatStringsSep " " (lib.mapAttrsToList (
      name: value: "--set ${escapeShellArg name} ${escapeShellArg value}"
    )
    environment);
  runtimeConfigCheck = lib.optionalString (cfg.runtimeConfigFile != null) ''
    roborev_runtime=${escapeShellArg cfg.runtimeConfigFile}
    if [[ -L "$roborev_runtime" || ! -f "$roborev_runtime" || ! -r "$roborev_runtime" || "$(${pkgs.coreutils}/bin/stat -c '%u:%a' "$roborev_runtime")" != "$(${pkgs.coreutils}/bin/id -u):600" ]]; then
      echo "roborev: runtimeConfigFile must be a readable, user-owned regular file with mode 0600" >&2
      exit 1
    fi
  '';
  telemetry =
    if cfg.enableTelemetry
    then "1"
    else "0";

  wrappedPackage = pkgs.symlinkJoin {
    name = "roborev-home-manager";
    paths = [cfg.package];
    nativeBuildInputs = [pkgs.makeWrapper];
    postBuild = ''
      mkdir -p "$out/libexec/roborev-agents"
      ${lib.concatStringsSep "\n" (lib.mapAttrsToList (
          name: path:
            lib.optionalString (builtins.hasAttr name commandNames)
            "makeWrapper ${escapeShellArg path} \"$out/libexec/roborev-agents/${commandNames.${name}}\""
        )
        cfg.agentCommands)}
      wrapProgram "$out/bin/roborev" \
        --set ROBOREV_DATA_DIR ${escapeShellArg cfg.dataDir} \
        --set ROBOREV_TELEMETRY_ENABLED ${escapeShellArg telemetry} \
        --set PATH "$out/libexec/roborev-agents":${escapeShellArg baseRuntimePath} \
        --run ${escapeShellArg runtimeConfigCheck} \
        ${environmentFlags}
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
      type = types.str;
      default = "opencode";
      description = "Explicit built-in or acp.<name> agent. Generated configuration requires a pinned executable; runtimeConfigFile owns agent selection in runtime-file mode.";
    };

    agentCommands = mkOption {
      type = types.attrsOf types.str;
      default = {};
      description = ''
        Mapping of any supported built-in review agent to an absolute executable path.
        Values are single executable paths, not shell commands with arguments.
        Pass an existing approved containment wrapper when one is in use.
        These are execution choices, not a security allowlist.
      '';
    };

    settings = mkOption {
      type = schema.globalType;
      default = {};
      description = ''
        Typed, non-secret upstream TOML settings with a freeform escape hatch.
        default_agent is owned by defaultAgent. Command settings may be supplied
        directly; when agentCommands supplies the same agent they must agree.
        Use runtime file paths or upstream-supported environment references
        for credentials. Never put plaintext secrets in this attribute set.
      '';
    };

    runtimeConfigFile = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = ''
        Complete private runtime TOML file, outside the Nix store. Its contents
        are never read during evaluation or builds. This mode supports config-only
        secrets such as auth_key and webhook URLs. It is mutually exclusive with
        settings and agentCommands; use extraPackages for runtime dependencies.
        The runtime file must be owned by this user and mode 0600.
      '';
    };

    colorMode = mkOption {
      type = types.nullOr (types.enum ["auto" "dark" "light" "none"]);
      default = null;
      description = "Roborev color mode. Null inherits the caller's environment; NO_COLOR retains upstream precedence.";
    };

    environment = mkOption {
      type = types.attrsOf types.str;
      default = {};
      description = "Non-secret environment shared by the wrapped CLI and service. Credentials belong in runtime files or service environmentFiles.";
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
      assertions =
        [
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
            message = "programs.roborev.agentCommands accepts only supported built-in review agents; configure named ACP commands under settings.acp.";
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
            assertion =
              cfg.runtimeConfigFile
              != null
              || (builtins.elem cfg.defaultAgent knownAgents && (builtins.hasAttr cfg.defaultAgent cfg.agentCommands || absoluteString configuredCommand))
              || (hasPrefix "acp." cfg.defaultAgent && acpName != "" && !hasInfix "." acpName && lib.trim acpName == acpName && absoluteString acpCommand);
            message = "The roborev defaultAgent requires an explicit agentCommands entry, absolute upstream command override, or defined acp.<name> command.";
          }
          {
            assertion = !(settings ? default_agent) && commandConflicts == [];
            message = "Use roborev.defaultAgent for default_agent; settings command overrides and agentCommands must agree (${concatStringsSep ", " commandConflicts}).";
          }
          {
            assertion = cfg.runtimeConfigFile == null || (runtimeSecretPath cfg.runtimeConfigFile && cfg.runtimeConfigFile != "${cfg.dataDir}/config.toml");
            message = "roborev.runtimeConfigFile must be an absolute runtime string outside the Nix store.";
          }
          {
            assertion = cfg.runtimeConfigFile == null || (settings == {} && cfg.agentCommands == {});
            message = "roborev.runtimeConfigFile is a complete configuration and cannot be combined with settings or agentCommands.";
          }
          {
            assertion = builtins.all (name: builtins.match "[A-Za-z_][A-Za-z0-9_]*" name != null) (builtins.attrNames cfg.environment);
            message = "roborev.environment names must be valid environment variable identifiers.";
          }
          {
            assertion = builtins.all (name: !(builtins.elem name ["PATH" "HOME" "ROBOREV_DATA_DIR" "ROBOREV_TELEMETRY_ENABLED" "ROBOREV_COLOR_MODE"])) (builtins.attrNames cfg.environment);
            message = "Use dedicated roborev options for managed environment variables.";
          }
          {
            assertion = builtins.all (value: builtins.match ".*[[:cntrl:]].*" value == null) (builtins.attrValues cfg.environment);
            message = "roborev.environment values must not contain control characters.";
          }
        ]
        ++ (import ./credentials.nix {inherit lib settings runtimeSecretPath envReference;});

      programs.roborev.finalPackage = wrappedPackage;
      programs.roborev._configFile = configFile;
      programs.roborev._runtimePath = runtimePath;
      programs.roborev.settings = mkIf (cfg.runtimeConfigFile == null) {
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
        ${runtimeConfigCheck}
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
