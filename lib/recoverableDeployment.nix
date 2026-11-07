{lib}: let
  inherit (lib) mkOption types;

  # Fleetix deployment target identifiers are ascii slugs. Reject slashes,
  # whitespace and path traversal before a target reaches a unit name or path.
  targetPattern = "[a-z0-9]([a-z0-9-]*[a-z0-9])?";
  targetType = types.strMatching targetPattern;
  timeoutType = types.ints.between 1 3600;
  commandType = types.listOf types.str;

  cleanAbsolute = value:
    builtins.isString value
    && value != "/"
    && lib.hasPrefix "/" value
    && builtins.match "/[^[:space:]%$]+" value != null
    && !(lib.hasInfix "//" value)
    && !(lib.hasSuffix "/" value)
    && lib.all (part: part != "." && part != "..") (lib.splitString "/" value);

  executableCommand = command:
    command
    != []
    && cleanAbsolute (builtins.head command)
    && lib.all (argument: !(lib.hasInfix "\n" argument)) command;

  underDirectory = directory: path:
    path == directory || lib.hasPrefix (directory + "/") path;

  runServiceName = name: "recoverable-deployment-${name}";
  recoverServiceName = name: "recoverable-deployment-${name}-recover";

  # The service must outlive up to nine retention calls, trial, commit and
  # restoration, plus three health windows and child-reaping grace. Keep the
  # original two-hour floor so a default policy still has room for a full
  # NixOS activation, and grow with operator-configured deadlines.
  timeoutStartSeconds = commandTimeoutSeconds: healthTimeoutSeconds:
    lib.max 7200 (16 * commandTimeoutSeconds + 3 * healthTimeoutSeconds + 180);

  binary = package:
    if package == null
    then throw "Recoverable deployment requires an explicit Fleetix package with fleetix-deploy."
    else "${package}/bin/fleetix-deploy";

  runArguments = {
    binary,
    policyFile,
    requestPath,
  }: [
    binary
    "run"
    "--policy"
    policyFile
    "--request"
    requestPath
  ];

  recoverArguments = {
    binary,
    policyFile,
  }: [
    binary
    "recover"
    "--policy"
    policyFile
  ];

  # Fixed worker policy. Every key is camelCase to match Fleetix's serde wire
  # contract. There is deliberately no caller-controlled approval field.
  mkPolicy = {
    target,
    backend,
    stateRoot,
    lockPath,
    profilePath,
    currentSystemPath,
    nixStore,
    nixEnv,
    admissionCommand,
    healthCommand,
    commandTimeoutSeconds,
    healthTimeoutSeconds,
    healthIntervalSeconds,
  }: {
    version = 1;
    inherit
      target
      backend
      stateRoot
      lockPath
      profilePath
      currentSystemPath
      nixStore
      nixEnv
      admissionCommand
      healthCommand
      commandTimeoutSeconds
      healthTimeoutSeconds
      healthIntervalSeconds
      ;
  };

  mkDeploymentOptions = {
    name,
    config,
    pkgs,
    defaultStateRoot,
    defaultLockPath,
    policyFor,
  }: {
    options = {
      enable = lib.mkEnableOption "recoverable deployment ${name}";

      package = mkOption {
        type = types.nullOr types.package;
        default = null;
        description = ''
          Fleetix deployment worker package. No Fleetix package default is
          selected: set an explicit package whose `bin/fleetix-deploy` was
          built with the `deployment-cli` feature.
        '';
      };

      target = mkOption {
        type = targetType;
        default = name;
        description = "Target identifier bound into the plan, admission receipt and policy.";
      };

      stateRoot = mkOption {
        type = types.str;
        default = defaultStateRoot;
        description = "Private persistent worker journal, policy and GC-root directory.";
      };

      lockPath = mkOption {
        type = types.str;
        default = defaultLockPath;
        description = "Absolute shared activation-lock anchor. All deployment frontends must use the same anchor.";
      };

      requestPath = mkOption {
        type = types.str;
        default = "";
        description = "Absolute writable request path watched by the optional path unit. The request is untrusted.";
      };

      watchRequest = mkOption {
        type = types.bool;
        default = true;
        description = "Run queued requests at startup and start the worker when the request changes.";
      };

      admissionCommand = mkOption {
        type = commandType;
        default = [];
        description = "Absolute trusted admission verifier argv. Fixed in policy; requests cannot supply it.";
      };

      healthCommand = mkOption {
        type = commandType;
        default = [];
        description = "Absolute trusted post-activation health argv. Fixed in policy; requests cannot supply it.";
      };

      commandTimeoutSeconds = mkOption {
        type = timeoutType;
        default = 30;
        description = "Deadline for each worker Nix or activation command.";
      };

      healthTimeoutSeconds = mkOption {
        type = timeoutType;
        default = 30;
        description = "Bounded acceptance window for consecutive successful health observations.";
      };

      healthIntervalSeconds = mkOption {
        type = timeoutType;
        default = 5;
        description = "Delay between trusted health command invocations.";
      };

      policy = mkOption {
        type = types.attrs;
        readOnly = true;
        default = policyFor config;
        description = "Rendered Fleetix policy object.";
      };

      policyFile = mkOption {
        type = types.str;
        readOnly = true;
        default = "${pkgs.writeText "fleetix-deployment-${name}.json" (builtins.toJSON config.policy)}";
        description = "Immutable store path of the trusted policy passed to the worker.";
      };
    };
  };

  assertions = {
    name,
    deployment,
  }: [
    {
      assertion = builtins.match targetPattern name != null && builtins.stringLength name <= 128;
      message = "Recoverable deployment '${name}' has an invalid option name.";
    }
    {
      assertion = !(lib.hasSuffix "-recover" name);
      message = "Recoverable deployment '${name}' must not end in '-recover'; that suffix is reserved for its recovery unit.";
    }
    {
      assertion = builtins.match targetPattern deployment.target != null && builtins.stringLength deployment.target <= 128;
      message = "Recoverable deployment '${name}' has an invalid target.";
    }
    {
      assertion = deployment.package != null;
      message = "Recoverable deployment '${name}' requires an explicit package; no Fleetix package default is selected.";
    }
    {
      assertion = executableCommand deployment.admissionCommand;
      message = "Recoverable deployment '${name}' requires an absolute trusted admission command.";
    }
    {
      assertion = executableCommand deployment.healthCommand;
      message = "Recoverable deployment '${name}' requires an absolute trusted health command.";
    }
    {
      assertion = cleanAbsolute deployment.stateRoot;
      message = "Recoverable deployment '${name}' requires an absolute private state root.";
    }
    {
      assertion = cleanAbsolute deployment.lockPath;
      message = "Recoverable deployment '${name}' requires an absolute shared lock path.";
    }
    {
      assertion = cleanAbsolute deployment.requestPath;
      message = "Recoverable deployment '${name}' requires an absolute request path.";
    }
    {
      assertion = deployment.healthIntervalSeconds < deployment.healthTimeoutSeconds;
      message = "Recoverable deployment '${name}' needs room for consecutive health observations.";
    }
    {
      assertion = !(underDirectory deployment.stateRoot deployment.requestPath);
      message = "Recoverable deployment '${name}' must keep untrusted requests outside private journal state.";
    }
  ];
in {
  inherit
    underDirectory
    runServiceName
    recoverServiceName
    timeoutStartSeconds
    binary
    runArguments
    recoverArguments
    mkPolicy
    mkDeploymentOptions
    assertions
    ;
}
