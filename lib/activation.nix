# Shared activation contracts for NixOS and Home Manager modules.
#
# The helper describes requirements and outputs; it does not generate
# application-specific shell. Consumers can expose a readiness contract
# without moving fleet policy or secret material into canix-toolbelt.
{lib}: let
  inherit (lib) filterAttrs mapAttrsToList;

  evaluationRequirements = requirements:
    filterAttrs (_: requirement:
      (requirement.phase or "evaluation")
      == "evaluation"
      && (requirement.severity or "error") == "error"
      && !(requirement.satisfied or false))
    requirements;

  requirementMessage = contractName: requirementName: requirement:
    lib.concatStringsSep " " [
      (contractName + ": requirement " + requirementName + " is not satisfied.")
      (requirement.summary or "")
      ("Producer: " + (requirement.producer or requirement.producerId or "unspecified") + ".")
      ("Recovery: " + (requirement.recovery or "unspecified") + ".")
      ("Validation: " + (requirement.validation or "unspecified") + ".")
    ];

  contractStatus = {
    enabled,
    requirements,
  }: let
    failures = evaluationRequirements requirements;
    unsatisfied = lib.any (requirement: !(requirement.satisfied or false)) (builtins.attrValues requirements);
  in {
    status =
      if !enabled
      then "disabled"
      else if failures != {}
      then "blocked"
      else if unsatisfied
      then "deferred"
      else "ready";
    declaredStatus =
      if !enabled
      then "disabled"
      else if failures != {}
      then "blocked"
      else if unsatisfied
      then "pending"
      else "declared-ready";
  };

  # The manifest is deliberately made only from declarative contract data.
  # It contains paths and identifiers, never secret contents.
  mkManifest = contracts: {
    schemaVersion = 2;
    contracts = lib.mapAttrs (id: contract: contract // {inherit id;}) contracts;
  };
in {
  # Returns the data and assertions a module can merge into its config.
  # Requirement values are plain attrsets so this helper works from both
  # NixOS and Home Manager modules.
  mkContract = {
    name,
    enabled ? true,
    requirements ? {},
    artifacts ? [],
    units ? [],
    schemaVersion ? 2,
    owner ? "",
    rollout ? "declarative",
    dependencies ? [],
    healthChecks ? [],
  }: let
    failures = evaluationRequirements requirements;
    status = contractStatus {inherit name enabled requirements;};
  in {
    inherit name enabled requirements artifacts units schemaVersion owner rollout dependencies healthChecks;
    inherit (status) status declaredStatus;

    assertions =
      mapAttrsToList (requirementName: requirement: {
        assertion = !enabled || (requirement.satisfied or false);
        message = requirementMessage name requirementName requirement;
      })
      failures;
  };

  inherit mkManifest;
}
