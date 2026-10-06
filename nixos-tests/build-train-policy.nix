# Lightweight parity fixture generated from the actual NixOS module without
# instantiating a host or realizing a package.
let
  cfg = {
    enable = true;
    builder = "atlas";
    user = "can";
    package.executable = "/tools/toolbelt";
    admissionContract = "qualified-resource-policy";
    workers = 2;
    planningWorkers = 1;
    queueLimit = 128;
    agingSeconds = 300;
    planningTimeoutSeconds = 180;
    workerTimeoutSeconds = 21600;
    queryTimeoutSeconds = 60;
    substitutes = true;
    memoryMax = "1G";
  };
  module = import ../modules/nixos/services/build-train.nix {
    config = {
      canix-toolbelt.services.buildTrain = cfg;
      nix.package.executable = "/tools/nix";
      networking.hostName = "atlas";
      users.users.can = {};
    };
    lib = {
      getExe = p: p.executable;
      mkIf = condition: content: assert condition; content;
      max = a: b:
        if a > b
        then a
        else b;
    };
    pkgs = {
      coreutils = "/tools/coreutils";
      stdenv.hostPlatform = {
        system = "x86_64-linux";
        isLinux = true;
      };
      writeText = _: text: text;
    };
  };
  command = module.config.systemd.services.fleetix-build-train.serviceConfig.ExecStart;
  prefix = "${cfg.package.executable} build-train serve --config ";
  service = builtins.fromJSON (builtins.substring (builtins.stringLength prefix) (-1) command);
  expected = builtins.fromJSON (builtins.readFile ../tests/fixtures/build-train-service.json);
in
  assert service == expected; service
