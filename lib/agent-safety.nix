# Backend-neutral command safety declarations.
#
# `autoSafe` is a prompt-friction policy only.  It is deliberately separate
# from semantic destructive-command guards and OS sandboxing.
{lib}: let
  inherit (lib) mkOption types;

  autoSafeSubmodule = types.submodule {
    options = {
      commands = mkOption {
        type = types.either (types.enum ["*"]) (types.listOf types.str);
        default = [];
        description = "Command patterns relative to each declared executable.";
      };

      executables = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Executable names or aliases. Empty means the program name.";
      };

      wrappers = mkOption {
        type = types.submodule {
          options = {
            environmentAssignments = mkOption {
              type = types.bool;
              default = true;
              description = "Allow environment-assignment prefixes in generated rules.";
            };
            nixStorePaths = mkOption {
              type = types.bool;
              default = true;
              description = "Allow /nix/store/.../bin/<executable> wrapper paths.";
            };
            resultBinPaths = mkOption {
              type = types.bool;
              default = true;
              description = "Allow ./result/bin/<executable> wrapper paths.";
            };
          };
        };
        default = {};
        description = "Renderer-specific executable wrapper policy.";
      };

      rationale = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Human-readable reason this command is prompt-safe.";
      };
    };
  };

  autoSafeType =
    types.coercedTo
    (types.either types.str (types.listOf types.str))
    (value: {commands = value;})
    autoSafeSubmodule;

  commandPatterns = value:
    if value.commands == "*"
    then ["*"]
    else value.commands;

  executableNames = name: value:
    if (value.executables or []) == []
    then [name]
    else value.executables;

  mkCommand = executable: command:
    if command == null
    then executable
    else "${executable} ${command}";

  commandVariants = command:
    if command == "*"
    then [null "*"]
    else [command];

  mkPathCommand = pathPrefix: executable: command: let
    suffix =
      if command == null
      then ""
      else " ${command}";
  in "${pathPrefix}/${executable}${suffix}";

  addRule = rules: name: action:
    if builtins.hasAttr name rules
    then assert lib.assertMsg (rules.${name} == action) "agent-safety: conflicting actions for `${name}` (${rules.${name}} vs ${action})"; rules
    else rules // {${name} = action;};

  normalize = name: value:
    if value == null
    then null
    else let
      normalizedValue =
        if builtins.isAttrs value
        then value
        else {commands = value;};
    in {
      inherit name;
      commands = commandPatterns normalizedValue;
      executables = executableNames name normalizedValue;
      wrappers = let
        rawWrappers = normalizedValue.wrappers or {};
      in {
        environmentAssignments = rawWrappers.environmentAssignments or true;
        nixStorePaths = rawWrappers.nixStorePaths or true;
        resultBinPaths = rawWrappers.resultBinPaths or true;
      };
      rationale = normalizedValue.rationale or "programs.${name}.autoSafe";
    };

  renderProgram = {
    name,
    value,
    action ? "allow",
  }: let
    normalized = normalize name value;
    commandRules =
      lib.concatMap (
        executable:
          lib.concatMap (command: [
            {
              key = mkCommand executable command;
              inherit action;
            }
          ])
          (lib.concatMap commandVariants normalized.commands)
      )
      normalized.executables;
    envRules = lib.optionals normalized.wrappers.environmentAssignments (
      lib.concatMap (
        executable:
          map (command: {
            key = "*=* ${mkCommand executable command}";
            inherit action;
          })
          (lib.concatMap commandVariants normalized.commands)
      )
      normalized.executables
    );
    storeRules = lib.optionals normalized.wrappers.nixStorePaths (
      lib.concatMap (
        executable:
          map (command: {
            key = mkPathCommand "/nix/store/*/bin" executable command;
            inherit action;
          })
          (lib.concatMap commandVariants normalized.commands)
      )
      normalized.executables
    );
    resultRules = lib.optionals normalized.wrappers.resultBinPaths (
      lib.concatMap (
        executable:
          map (command: {
            key = mkPathCommand "./result/bin" executable command;
            inherit action;
          })
          (lib.concatMap commandVariants normalized.commands)
      )
      normalized.executables
    );
  in
    commandRules ++ envRules ++ storeRules ++ resultRules;

  renderPrograms = programs:
    builtins.foldl' (
      rules: name: let
        value = programs.${name};
      in
        if value.autoSafe == null
        then rules
        else
          builtins.foldl' (acc: entry: addRule acc entry.key entry.action) rules
          (renderProgram {
            inherit name;
            value = value.autoSafe;
          })
    ) {} (builtins.attrNames programs);

  renderActionRules = {
    allow ? [],
    ask ? [],
    deny ? [],
  }: let
    add = action: patterns: rules:
      builtins.foldl' (acc: pattern: addRule acc pattern action) rules patterns;
  in
    add "deny" deny (add "ask" ask (add "allow" allow {}));

  stripEnvironmentPrefix = pattern: let
    match = builtins.match "\\*=\\*+ (.*)" pattern;
  in
    if match == null
    then pattern
    else builtins.elemAt match 0;

  patternPrefix = pattern: let
    match = builtins.match "([^*]*)\\*.*" (stripEnvironmentPrefix pattern);
  in
    if match == null
    then pattern
    else builtins.elemAt match 0;

  hasWildcard = pattern: builtins.match ".*\\*.*" (stripEnvironmentPrefix pattern) != null;

  patternsMayOverlap = left: right: let
    leftWildcard = hasWildcard left;
    rightWildcard = hasWildcard right;
    leftPrefix = patternPrefix left;
    rightPrefix = patternPrefix right;
    leftPattern = stripEnvironmentPrefix left;
    rightPattern = stripEnvironmentPrefix right;
  in
    if !leftWildcard && !rightWildcard
    then leftPattern == rightPattern
    else if leftWildcard && rightWildcard
    then lib.hasPrefix leftPrefix rightPrefix || lib.hasPrefix rightPrefix leftPrefix
    else if leftWildcard
    then lib.hasPrefix leftPrefix rightPattern
    else lib.hasPrefix rightPrefix leftPattern;

  mergeRules = {
    defaultAction ? "ask",
    programs ? {},
    allow ? [],
    ask ? [],
    deny ? [],
  }: let
    programRules = renderPrograms programs;
    policyRules = renderActionRules {inherit allow ask deny;};
    conflicts =
      lib.filter (
        programPattern:
          builtins.any (denyPattern: patternsMayOverlap programPattern denyPattern) deny
      )
      (builtins.attrNames programRules);
  in
    assert lib.assertMsg (conflicts == []) (
      "agent-safety: autoSafe cannot override deny rule(s): "
      + lib.concatStringsSep ", " conflicts
    );
      {
        "*" = defaultAction;
      }
      // programRules
      // policyRules;
in {
  inherit
    autoSafeSubmodule
    autoSafeType
    mergeRules
    normalize
    renderActionRules
    renderProgram
    renderPrograms
    ;

  mkAutoSafeOption = description:
    mkOption {
      type = types.nullOr autoSafeType;
      default = null;
      description = ''
        ${description}

        This controls interactive prompt friction only. It does not replace
        destructive-command guards or OS sandboxing.
      '';
    };
}
