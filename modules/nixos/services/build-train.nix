# Fleetix owns the service, policy renderer and lifecycle. Preserve Toolbelt's
# public option paths using leaf aliases.
{fleetixBuildTrain}: {lib, ...}: {
  imports =
    [fleetixBuildTrain]
    ++ map
    (name:
      lib.mkAliasOptionModule
      ["canix-toolbelt" "services" "buildTrain" name]
      ["fleetix" "services" "buildTrain" name])
    [
      "enable"
      "package"
      "user"
      "serviceScope"
      "builder"
      "admissionContract"
      "runtimeDirectory"
      "stateDirectory"
      "gcRoots"
      "workers"
      "queueLimit"
      "agingSeconds"
      "workerTimeoutSeconds"
      "queryTimeoutSeconds"
      "planningWorkers"
      "planningTimeoutSeconds"
      "substitutes"
      "memoryMax"
      "fenceSeconds"
      # Removed upstream; the alias forwards its removal notice.
      "retainedDeployment"
    ];
}
