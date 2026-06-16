# Pure helpers for opencode permission rule DSL: env-prefix twin expansion,
# git subcommand pairing, and bash permission action validation.
#
# Consumers feed in bash permission rules and receive the expanded set
# with twin rules (env-prefix and git -C variants). Host-agnostic.
{ lib }: let
  gitPair = subcmd: perm: {
    "git ${subcmd}" = perm;
    "git -C * ${subcmd}" = perm;
  };

  validBashPermissionActions = ["allow" "ask" "deny"];
  invalidBashPermissionActions = rules:
    lib.filterAttrs (_: value: !(lib.elem value validBashPermissionActions)) rules;
  validateBashPermissionActions = rules: let
    invalid = invalidBashPermissionActions rules;
    formatInvalid = name: value: "${name}=${value}";
  in
    assert lib.assertMsg (invalid == {}) ''
      opencode permission.bash contains invalid action(s): ${
        lib.concatStringsSep ", " (lib.mapAttrsToList formatInvalid invalid)
      }
    ''; rules;

  envTwinPrefix = action:
    if action == "allow"
    then "*=*"
    else if action == "ask"
    then "*=**"
    else "*=***";

  withEnvPrefixes = rules: let
    validatedRules = validateBashPermissionActions rules;
  in
    validatedRules
    // lib.mapAttrs' (name: value: lib.nameValuePair "${envTwinPrefix value} ${name}" value)
    (lib.filterAttrs (name: _: name != "*" && !lib.hasPrefix "/" name) validatedRules);
in {
  inherit
    gitPair
    validBashPermissionActions
    invalidBashPermissionActions
    validateBashPermissionActions
    envTwinPrefix
    withEnvPrefixes;
}
