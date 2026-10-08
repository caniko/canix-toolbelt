# Fleetix owns the service, policy renderer and lifecycle. Preserve Toolbelt's
# public option paths using leaf aliases, including staged deployment recovery.
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
      "retainedDeployment"
    ];
}
