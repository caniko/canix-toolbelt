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
      map (rule: rule // {key = "*=* ${rule.key}";})
      (commandRules ++ storeRules ++ resultRules)
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
    # Backend wildcards span slashes, spaces and arguments. A shared store
    # prefix does not bound its wildcard to the directory hash: stripping it
    # can miss overlaps between different executable suffixes in one command.
    leftPattern = stripEnvironmentPrefix left;
    rightPattern = stripEnvironmentPrefix right;
    leftWildcard = hasWildcard leftPattern;
    rightWildcard = hasWildcard rightPattern;
    leftPrefix = patternPrefix leftPattern;
    rightPrefix = patternPrefix rightPattern;
  in
    if !leftWildcard && !rightWildcard
    then leftPattern == rightPattern
    else if leftWildcard && rightWildcard
    then lib.hasPrefix leftPrefix rightPrefix || lib.hasPrefix rightPrefix leftPrefix
    else if leftWildcard
    then lib.hasPrefix leftPrefix rightPattern
    else lib.hasPrefix rightPrefix leftPattern;

  # Codex execpolicy matches literal argv prefixes and resolves overlapping
  # rules strictest-wins, so only literal patterns translate exactly. Exact
  # allows are dropped because a prefix rule would widen them, and quoted,
  # escaped or expanded words never equal Codex's parsed argv.
  # ponytail: an ask/deny glob degrades to `prompt` on its literal prefix, so
  # OpenCode-allowed refinements under it (for example IFD-guarded nix
  # evaluations) prompt in Codex; per-argument alternatives would recover them.
  #
  # Codex may resolve an absolute program path through a bare-name rule. Each
  # allowed bare name is pinned to `executableDirs`, so a workspace or store
  # binary sharing the name falls through to the approval policy instead.
  renderCodexRules = {
    rules,
    executableDirs ? [],
  }: let
    decisions = {
      allow = "allow";
      ask = "prompt";
      deny = "forbidden";
    };
    # Shell operators split Codex segments, so they end a prefix like a glob.
    isAssignment = token: builtins.match "[A-Za-z_][A-Za-z0-9_]*=.*" token != null;
    isLiteral = token: builtins.match ".*[][*?'\"\\$`{}~|&;<>()].*" token == null;
    analyse = pattern: let
      # The shell drops a word-initial `#` comment and never yields the empty
      # words that repeated spaces would give.
      command = builtins.head (builtins.split "(^|[[:space:]])#" pattern);
      tokens = lib.filter (token: builtins.isString token && token != "") (builtins.split "[[:space:]]+" command);
      trailing = tokens != [] && lib.last tokens == "*";
      body =
        if trailing
        then lib.init tokens
        else tokens;
      # A leading `NAME=value` is a shell assignment, never an argv word.
      assigned = tokens != [] && isAssignment (builtins.head tokens);
      literal =
        if assigned
        then []
        else
          (builtins.foldl' (acc: token:
              if acc.stop || !isLiteral token
              then acc // {stop = true;}
              else acc // {tokens = acc.tokens ++ [token];}) {
              stop = false;
              tokens = [];
            }
            body).tokens;
    in {
      inherit literal trailing assigned;
      empty = tokens == [];
      exact = literal == body;
    };
    analysed = lib.mapAttrs (pattern: _: analyse pattern) rules;
    # Only refinements that render count: an exact or globbed allow is dropped,
    # so it must not weaken a deny it could never re-allow.
    lenient = lib.concatMap (pattern: let
      entry = analysed.${pattern};
    in
      lib.optional (pattern
        != "*"
        && entry.literal != []
        && (rules.${pattern} == "ask" || (rules.${pattern} == "allow" && entry.exact && entry.trailing)))
      entry)
    (builtins.attrNames rules);
    # OpenCode lets a longer allow/ask (or `x *` against an exact `x`) refine
    # a deny; Codex resolves overlaps strictest-wins, so such a deny is capped
    # at prompt instead of blocking the refinement.
    refined = literal: trailing: let
      length = builtins.length literal;
    in
      lib.any (other:
        lib.take length other.literal
        == literal
        && (builtins.length other.literal > length || (builtins.length other.literal == length && (!other.exact || (!trailing && other.trailing)))))
      lenient;
    render = pattern: let
      action = rules.${pattern};
      inherit (analysed.${pattern}) literal trailing exact;
      decision =
        if action == "allow"
        then
          if exact && trailing
          then "allow"
          else null
        else if exact && !(action == "deny" && refined literal trailing)
        then decisions.${action}
        else "prompt";
      # Codex never reduces an env-assigned script to its inner command, and
      # it has no glob for program names. A wrapper ask/deny whose bare twin
      # (or the catch-all) is at least as strict is dropped; any other
      # ask/deny without a literal prefix is listed as unrepresentable.
      strictness = {
        allow = 0;
        ask = 1;
        deny = 2;
      };
      unwrapped = lib.removePrefix "/nix/store/*/bin/" (stripEnvironmentPrefix pattern);
      bareAction = rules.${unwrapped} or rules."*" or "allow";
      coveredWrapper = unwrapped != pattern && strictness.${bareAction} >= strictness.${action};
    in
      if decision == null || pattern == "*" || analysed.${pattern}.empty
      then []
      else if literal != []
      then [
        {
          inherit decision;
          tokens = literal;
        }
      ]
      else if coveredWrapper
      then []
      else [{unrepresentable = "${action} ${pattern}";}];
    results = lib.concatMap render (builtins.attrNames rules);
    # Codex evaluates a shell script it cannot reduce to plain commands (for
    # example one with variable assignments or `if`/`for` control flow) as the
    # whole `bash -lc` argv, so no bare rule reaches `FOO=1 sudo` or
    # `if true; then rm -rf /; fi`. Prompt for those scripts instead.
    # ponytail: this degrades such denies to prompts and prompts every
    # unreducible script; per-script rules would need Codex-side parsing.
    guarded = lib.any (pattern: pattern != "*" && rules.${pattern} != "allow") (builtins.attrNames rules);
    shellPrompt = lib.optional guarded ''prefix_rule(pattern=[["bash", "sh", "zsh"], ["-c", "-lc"]], decision="prompt")'';
    rendered = lib.filter (result: result ? tokens) results;
    unrepresentable = map (result: result.unrepresentable) (lib.filter (result: result ? unrepresentable) results);
    quote = lib.concatMapStringsSep ", " builtins.toJSON;
    # Only allowed names are pinned, so a deny-only name still matches any
    # path. ponytail: a name with both allows and denies is pinned, so its
    # denies reach an unregistered copy only through the approval policy.
    bareNames = lib.unique (lib.filter (name: !(lib.hasInfix "/" name)) (map (rule: builtins.head rule.tokens) (lib.filter (rule: rule.decision == "allow") rendered)));
    prefixRules = map (rule: "prefix_rule(pattern=[${quote rule.tokens}], decision=${builtins.toJSON rule.decision})") rendered;
    hostExecutables = map (name: "host_executable(name=${builtins.toJSON name}, paths=[${quote (map (dir: "${dir}/${name}") executableDirs)}])") bareNames;
    # Kept visible rather than silently dropped; Codex's approval policy and
    # sandbox govern these commands instead.
    notes =
      lib.optional (unrepresentable != []) "# Not representable as Codex execpolicy rules:"
      ++ map (entry: "#   ${builtins.replaceStrings ["\n"] [" "] entry}") (lib.sort lib.lessThan unrepresentable);
  in
    lib.concatMapStrings (line: line + "\n")
    (lib.unique (lib.sort lib.lessThan (prefixRules ++ hostExecutables)) ++ shellPrompt ++ notes);

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
    renderCodexRules
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
