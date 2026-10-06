{
  pkgs,
  homeManager,
  package,
  homeDirectory,
  username,
  agentCommand,
}: let
  generation = settings: (homeManager.lib.homeManagerConfiguration {
    inherit pkgs;
    modules = [
      ../../modules/home/roborev
      {
        home = {
          inherit username homeDirectory;
          stateVersion = "26.05";
        };
        programs.roborev = {
          enable = true;
          inherit package settings;
          dataDir = "${homeDirectory}/state $literal %pct \"quote\" \\back";
          agentCommands.opencode = agentCommand;
        };
      }
    ];
  });
  first = generation {};
  changed = generation {max_workers = 3;};
in {
  dataDir = "${homeDirectory}/state $literal %pct \"quote\" \\back";
  package = {
    derivation = package.drvPath;
    output = toString package;
  };
  generations = map (value: {
    derivation = value.activationPackage.drvPath;
    output = toString value.activationPackage;
    wrapper = toString value.config.programs.roborev.finalPackage;
  }) [first changed];
}
