{lib}: let
  primaryClasses = ["owned" "forks" "upstream"];
  protectedClasses = ["personal" "local" "incubator"];
  clean = label: value:
    if
      value
      == ""
      || lib.hasPrefix "/" value
      || lib.hasSuffix "/" value
      || lib.any (segment: builtins.elem segment ["" "." ".."]) (lib.splitString "/" value)
    then throw "canix-toolbelt projectTree: ${label} must be a non-empty relative segment/path"
    else value;
  cleanSegment = label: value:
    if value == "." || value == ".." || builtins.match "[^/]+" value == null
    then throw "canix-toolbelt projectTree: ${label} must be one non-empty path segment"
    else value;
  coordinate = {
    forge,
    namespace,
    repository,
  }: "${clean "forge" forge}/${clean "namespace" namespace}/${clean "repository" repository}";
in rec {
  schemaVersion = 1;
  inherit primaryClasses protectedClasses;
  layout = {
    primary = {
      owned = "repos/owned";
      forks = "forks";
      upstream = "upstream";
    };
    worktrees = "worktrees";
    protected = {
      personal = "personal";
      local = "local";
      incubator = "incubator";
    };
    ignored = ["archives"];
  };

  inherit coordinate;

  projectPath = {
    class,
    repository,
    ...
  }:
    if !builtins.elem class primaryClasses
    then throw "canix-toolbelt projectTree: unsupported primary class ${class}"
    else "${layout.primary.${class}}/${cleanSegment "repository" repository}";

  worktreePath = {
    repository,
    purpose,
    ...
  }: "${layout.worktrees}/${cleanSegment "repository" repository}/${clean "purpose" purpose}";
}
