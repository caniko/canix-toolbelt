{
  config,
  lib,
  ...
}: let
  inherit (lib) mkOption types mapAttrsToList concatLists filterAttrs;

  checkModule = types.submodule {
    options = {
      name = mkOption {
        type = types.str;
        default = "";
        description = "Stable name for this runtime or health check.";
      };

      kind = mkOption {
        type = types.enum ["file" "secret" "directory" "unit" "tcp" "http" "topology" "hardware" "state" "backup" "manual-evidence" "exec"];
        default = "file";
        description = "Non-mutating check kind understood by the runtime checker.";
      };

      path = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Filesystem or evidence path for path-based checks.";
      };

      host = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Host name for TCP checks.";
      };

      port = mkOption {
        type = types.nullOr types.int;
        default = null;
        description = "TCP port for TCP checks.";
      };

      unit = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Systemd unit for unit checks.";
      };

      url = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "URL for HTTP checks.";
      };

      method = mkOption {
        type = types.enum ["GET" "HEAD"];
        default = "GET";
        description = "HTTP method for a non-mutating HTTP check.";
      };

      acceptedStatus = mkOption {
        type = types.listOf types.int;
        default = [200];
        description = "Accepted HTTP status codes.";
      };

      predicates = mkOption {
        type = types.listOf (types.enum ["exists" "readable" "non-empty" "writable" "active"]);
        default = [];
        description = "Predicates applied by path or unit checks.";
      };

      argv = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Argument vector for a narrowly-scoped executable check; no shell is allowed.";
      };
    };
  };

  dependencyModule = types.submodule {
    options = {
      target = mkOption {
        type = types.str;
        description = "Contract or logical workload identifier this contract depends on.";
      };

      phase = mkOption {
        type = types.enum ["evaluation" "activation" "runtime" "migration"];
        default = "runtime";
        description = "Lifecycle phase in which this dependency is required.";
      };

      required = mkOption {
        type = types.bool;
        default = true;
        description = "Whether this dependency is hard or merely wanted.";
      };
    };
  };

  requirementModule = types.submodule {
    options = {
      satisfied = mkOption {
        type = types.bool;
        default = false;
        description = "Whether this activation prerequisite is satisfied at evaluation time.";
      };

      phase = mkOption {
        type = types.enum ["evaluation" "activation" "runtime" "migration"];
        default = "evaluation";
        description = "Lifecycle phase at which this prerequisite can be checked.";
      };

      severity = mkOption {
        type = types.enum ["error" "warning"];
        default = "error";
        description = "Whether an unsatisfied prerequisite blocks evaluation.";
      };

      summary = mkOption {
        type = types.str;
        default = "";
        description = "Short explanation of the missing prerequisite.";
      };

      producer = mkOption {
        type = types.str;
        default = "unspecified";
        description = "Upstream module, data source, or workflow that produces the prerequisite.";
      };

      producerId = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Stable machine-readable producer identifier.";
      };

      kind = mkOption {
        type = types.enum ["evaluation" "secret" "file" "directory" "unit" "tcp" "http" "topology" "hardware" "state" "backup" "manual-evidence" "exec"];
        default = "evaluation";
        description = "Machine-readable prerequisite kind.";
      };

      sensitive = mkOption {
        type = types.bool;
        default = false;
        description = "Whether check output must be redacted.";
      };

      check = mkOption {
        type = types.nullOr checkModule;
        default = null;
        description = "Typed non-mutating check specification.";
      };

      recovery = mkOption {
        type = types.str;
        default = "unspecified";
        description = "Action that repairs or supplies the prerequisite.";
      };

      validation = mkOption {
        type = types.str;
        default = "unspecified";
        description = "Command or check proving that the prerequisite is present.";
      };
    };
  };

  artifactModule = types.submodule {
    options = {
      id = mkOption {
        type = types.str;
        default = "";
        description = "Stable artifact identifier.";
      };

      kind = mkOption {
        type = types.enum ["file" "directory" "unit" "manifest"];
        default = "file";
        description = "Kind of activation output.";
      };

      path = mkOption {
        type = types.str;
        description = "Path or identifier of the output.";
      };

      sensitive = mkOption {
        type = types.bool;
        default = false;
        description = "Whether the output must not be embedded into the Nix store.";
      };

      persistence = mkOption {
        type = types.enum ["runtime" "state" "backup" "generated"];
        default = "runtime";
        description = "Whether the artifact is runtime-only, persistent state, backup evidence, or generated data.";
      };

      producerId = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Stable producer identifier for this artifact.";
      };
    };
  };

  contractModule = types.submodule ({config, ...}: {
    options = {
      schemaVersion = mkOption {
        type = types.ints.positive;
        default = 2;
        description = "Activation contract wire-schema version.";
      };

      enabled = mkOption {
        type = types.bool;
        default = false;
        description = "Whether this feature is requested.";
      };

      requirements = mkOption {
        type = types.attrsOf requirementModule;
        default = {};
        description = "Typed prerequisites for this feature.";
      };

      artifacts = mkOption {
        type = types.listOf artifactModule;
        default = [];
        description = "Files, units, or manifests produced by activation.";
      };

      units = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Systemd units participating in this feature.";
      };

      owner = mkOption {
        type = types.str;
        default = "";
        description = "Repository/module owner of the contract mechanics.";
      };

      rollout = mkOption {
        type = types.enum ["declarative" "render" "reconcile" "migration"];
        default = "declarative";
        description = "Operational risk class for activation behavior.";
      };

      dependencies = mkOption {
        type = types.listOf dependencyModule;
        default = [];
        description = "Logical contract dependencies; these do not rewrite systemd ordering by themselves.";
      };

      healthChecks = mkOption {
        type = types.listOf checkModule;
        default = [];
        description = "Non-mutating checks proving the live workload is healthy.";
      };

      status = mkOption {
        type = types.enum ["disabled" "blocked" "deferred" "ready"];
        readOnly = true;
        default =
          if !config.enabled
          then "disabled"
          else if builtins.any (requirement: requirement.phase == "evaluation" && requirement.severity == "error" && !requirement.satisfied) (builtins.attrValues config.requirements)
          then "blocked"
          else if builtins.any (requirement: !requirement.satisfied) (builtins.attrValues config.requirements)
          then "deferred"
          else "ready";
        description = "Computed readiness state.";
      };

      declaredStatus = mkOption {
        type = types.enum ["disabled" "blocked" "pending" "declared-ready"];
        readOnly = true;
        default =
          if !config.enabled
          then "disabled"
          else if builtins.any (requirement: requirement.phase == "evaluation" && requirement.severity == "error" && !requirement.satisfied) (builtins.attrValues config.requirements)
          then "blocked"
          else if builtins.any (requirement: !requirement.satisfied) (builtins.attrValues config.requirements)
          then "pending"
          else "declared-ready";
        description = "Evaluation-only status; runtime requirements remain pending until observed on a host.";
      };
    };
  });

  contracts = config.canix-toolbelt.activation.contracts;
  assertions = concatLists (mapAttrsToList (
      name: contract:
        mapAttrsToList (requirementName: requirement: {
          assertion =
            !contract.enabled
            || requirement.phase != "evaluation"
            || requirement.severity != "error"
            || requirement.satisfied;
          message = lib.concatStringsSep " " [
            (name + ": requirement " + requirementName + " is not satisfied.")
            requirement.summary
            ("Producer: " + (if requirement.producerId != null then requirement.producerId else requirement.producer) + ".")
            ("Recovery: " + requirement.recovery + ".")
            ("Validation: " + requirement.validation + ".")
          ];
        }) (filterAttrs (
            _: requirement:
              requirement.phase == "evaluation" && requirement.severity == "error"
          )
          contract.requirements)
    )
    contracts);
in {
  options.canix-toolbelt.activation.contracts = mkOption {
    type = types.attrsOf contractModule;
    default = {};
    description = "Readiness and activation contracts exported by reusable modules.";
  };

  config.assertions = assertions;
}
