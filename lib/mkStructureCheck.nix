{lib}: {pkgs}: {
  name,
  src,
  rules,
  includeHidden ? false,
  noIgnore ? false,
  reportAll ? false,
}: let
  normalizedRules = map (rule:
    rule
    // {
      paths =
        if (rule.paths or []) == []
        then ["."]
        else rule.paths;
      globs = rule.globs or [];
      exclude = rule.exclude or [];
      pcre2 = rule.pcre2 or false;
    })
  rules;
  sourceRoot =
    if builtins.isPath src
    then src
    # fileset needs a path; its source restores a context-carrying flake dependency.
    else /. + builtins.unsafeDiscardStringContext (toString src);
  scanPaths = lib.unique (lib.concatMap (rule: map (path: sourceRoot + "/${path}") rule.paths) normalizedRules);
  ancestorIgnores = path:
    if path == sourceRoot
    then []
    else let
      parent = builtins.dirOf path;
      ignores = map (name: parent + "/${name}") [".gitignore" ".ignore" ".rgignore"];
    in
      lib.filter (ignore: builtins.pathExists ignore && builtins.readFileType ignore != "directory") ignores
      ++ ancestorIgnores parent;
  # Unrelated consumer files must not invalidate the selected source checks.
  # Subtrees include their own ignore files; only ancestor ignores are added.
  source = lib.fileset.toSource {
    root = sourceRoot;
    fileset = lib.fileset.unions (scanPaths ++ lib.concatMap ancestorIgnores scanPaths);
  };
  scanFlags = lib.optionalString includeHidden "--hidden " + lib.optionalString noIgnore "--no-ignore ";
  mkRule = rule: let
    globArgs = lib.concatMapStringsSep " " (glob: "-g ${lib.escapeShellArg glob}") (rule.globs ++ map (path: "!/${path}") rule.exclude);
    pathArgs = lib.concatMapStringsSep " " lib.escapeShellArg rule.paths;
    onMatch =
      if reportAll
      then ''if [ "$result" -eq 0 ]; then result=1; fi''
      else "exit 1";
    onError =
      if reportAll
      then ''result="$status"''
      else ''exit "$status"'';
  in ''
    echo ${lib.escapeShellArg ">> ${rule.name}"}
    if rg -n --no-require-git ${scanFlags}${lib.optionalString rule.pcre2 "-P "}${globArgs} -- ${lib.escapeShellArg rule.pattern} ${pathArgs}; then
      echo ${lib.escapeShellArg rule.message} >&2
      ${onMatch}
    else
      status=$?
      if [ "$status" -ne 1 ]; then
        echo ${lib.escapeShellArg "${rule.name}: source scanner failed"} >&2
        ${onError}
      fi
    fi
  '';
in
  assert lib.assertMsg (lib.all (path: lib.path.hasPrefix sourceRoot path && builtins.readFileType path == "directory") scanPaths)
  "${name}: source-rule paths must be directories within src; select files through their parent directory and globs";
    pkgs.runCommand name {nativeBuildInputs = [pkgs.ripgrep];} ''
      cd ${source}
      ${lib.optionalString reportAll "result=0"}
      ${lib.concatMapStringsSep "\n" mkRule normalizedRules}
      ${lib.optionalString reportAll ''if [ "$result" -ne 0 ]; then exit "$result"; fi''}
      touch "$out"
    ''
