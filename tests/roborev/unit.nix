{
  pkgs,
  homeManager,
  homeDirectory,
  agentCommand,
  environmentFile,
  requiredFile,
  settings ? {},
  extraPackages ? [],
  recorderRevision ? "baseline",
}: let
  # Protocol recorder only: no agent, repository, provider or credential access.
  recorder = pkgs.writeScriptBin "roborev" ''
    #!${pkgs.python3}/bin/python3
    # package-fixture-revision: ${recorderRevision}
    import json, os, shutil, signal, socket, subprocess, sys, time, tomllib
    from pathlib import Path
    data = Path(os.environ["ROBOREV_DATA_DIR"])
    arguments = sys.argv[1:]
    with (data / "invocations.jsonl").open("a") as log:
        log.write(json.dumps({"args": arguments, "home": os.environ["HOME"],
                              "data": str(data), "rotation": os.environ.get("RR_ROTATION"),
                              "telemetry": os.environ["ROBOREV_TELEMETRY_ENABLED"],
                              "path": os.environ["PATH"], "pid": os.getpid(),
                              "cwd": os.getcwd(),
                              "workers": tomllib.loads((data / "config.toml").read_text())["max_workers"],
                              "recorder_revision": ${builtins.toJSON recorderRevision},
                              "hello": shutil.which("hello"),
                              "tools": {name: shutil.which(name) for name in
                                        ("git", "gh", "ssh", "bash", "cat", "grep", "sed", "rg", "find")}}) + "\n")
    if arguments == ["config", "validate", "--global"]:
        tomllib.loads((data / "config.toml").read_text())
    elif arguments[:2] == ["daemon", "run"]:
        assert arguments == ["daemon", "run", "--config", str(data / "config.toml")]
        subprocess.Popen([sys.executable, "-c", "import time; time.sleep(3600)"])
        notify = os.environ["NOTIFY_SOCKET"]
        if notify.startswith("@"):
            notify = "\0" + notify[1:]
        with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as connection:
            connection.sendto(b"READY=1", notify)
        signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
        while True:
            time.sleep(1)
    else:
        sys.exit("unexpected fixture command")
  '';
  home = homeManager.lib.homeManagerConfiguration {
    inherit pkgs;
    modules = [
      ../../modules/home/roborev
      {
        home.username = "roborev-fixture";
        home.homeDirectory = homeDirectory;
        home.stateVersion = "26.05";
        systemd.user.startServices = "sd-switch";
        programs.roborev = {
          enable = true;
          inherit settings extraPackages;
          package = recorder;
          dataDir = "${homeDirectory}/state $literal %pct \"quote\" \\back";
          agentCommands.opencode = agentCommand;
        };
        services.roborev = {
          enable = true;
          environmentFiles = [environmentFile];
          requiredFiles = [requiredFile];
        };
      }
    ];
  };
  cfg = home.config;
  unit = cfg.xdg.configFile."systemd/user/roborev.service".source;
  # HM's source is a writeTextFile output plus /roborev.service, not a
  # derivation attribute set. Preserve that exact file and realize its context.
  unitDerivations = builtins.attrNames (builtins.getContext (toString unit));
in {
  dataDir = cfg.programs.roborev.dataDir;
  configFile = toString cfg.programs.roborev._configFile;
  wrappedPackage = toString cfg.programs.roborev.finalPackage;
  unitFile = toString unit;
  agentTarget = "${pkgs.coreutils}/bin/true";
  runtimePath = cfg.programs.roborev._runtimePath;
  inherit recorderRevision;
  expectHello = builtins.elem pkgs.hello extraPackages;
  helloCommand = "${pkgs.hello}/bin/hello";
  generation = toString home.activationPackage;
  generationDerivation = home.activationPackage.drvPath;
  derivations = assert unitDerivations != [] && builtins.all (path: pkgs.lib.hasSuffix ".drv" path) unitDerivations;
    (map (package: package.drvPath) [cfg.programs.roborev._configFile cfg.programs.roborev.finalPackage]) ++ unitDerivations;
}
