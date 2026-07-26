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
      owned = "owned";
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
    forge,
    namespace,
    repository,
  }:
    if !builtins.elem class primaryClasses
    then throw "canix-toolbelt projectTree: unsupported primary class ${class}"
    else "${layout.primary.${class}}/${coordinate {inherit forge namespace repository;}}";

  worktreePath = {
    forge,
    namespace,
    repository,
    purpose,
  }: "${layout.worktrees}/${coordinate {inherit forge namespace repository;}}/${clean "purpose" purpose}";
}
