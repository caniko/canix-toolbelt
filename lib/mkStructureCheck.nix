{lib}: {pkgs}: {
  name,
  src,
  rules,
}: let
  normalizedRules = map (rule:
    rule
    // {
      paths =
        if (rule.paths or []) == []
        then ["."]
        else rule.paths;
      globs = rule.globs or [];
    })
  rules;
  sourceRoot =
    if builtins.isPath src
    then src
    # fileset needs a path; its source restores a context-carrying flake dependency.
    else /. + builtins.unsafeDiscardStringContext (toString src);
  # Unrelated consumer files must not invalidate the selected source checks.
  source = lib.fileset.toSource {
    root = sourceRoot;
    fileset = lib.fileset.unions (
      [(lib.fileset.fileFilter (file: builtins.elem file.name [".gitignore" ".ignore" ".rgignore"]) sourceRoot)]
      ++ lib.concatMap (rule: map (path: sourceRoot + "/${path}") rule.paths) normalizedRules
    );
  };
  mkRule = rule: let
    globArgs = lib.concatMapStringsSep " " (glob: "-g ${lib.escapeShellArg glob}") rule.globs;
    pathArgs = lib.concatMapStringsSep " " lib.escapeShellArg rule.paths;
  in ''
    echo ${lib.escapeShellArg ">> ${rule.name}"}
    if rg -n ${globArgs} -- ${lib.escapeShellArg rule.pattern} ${pathArgs}; then
      echo ${lib.escapeShellArg rule.message} >&2
      exit 1
    else
      status=$?
      if [ "$status" -ne 1 ]; then
        echo ${lib.escapeShellArg "${rule.name}: source scanner failed"} >&2
        exit "$status"
      fi
    fi
  '';
in
  pkgs.runCommand name {nativeBuildInputs = [pkgs.ripgrep];} ''
    cd ${source}
    ${lib.concatMapStringsSep "\n" mkRule normalizedRules}
    touch "$out"
  ''
