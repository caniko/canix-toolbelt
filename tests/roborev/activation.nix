{
  pkgs,
  homeManager,
  homeDirectory,
}: let
  cfg =
    (homeManager.lib.homeManagerConfiguration {
      inherit pkgs;
      modules = [
        ../../modules/home/roborev
        {
          home.username = "roborev-fixture";
          home.homeDirectory = homeDirectory;
          home.stateVersion = "26.05";
          programs.roborev = {
            enable = true;
            package = pkgs.hello;
            dataDir = "${homeDirectory}/state $literal %pct \"quote\"";
            agentCommands.opencode = "${pkgs.coreutils}/bin/true";
          };
          services.roborev.enable = true;
        }
      ];
    }).config;
in {
  guard = cfg.home.activation.roborevDataDirCheck.data;
  create = cfg.home.activation.roborevDataDir.data;
  dataDir = cfg.programs.roborev.dataDir;
  target = cfg.home.file.roborev-config.target;
  unit = cfg.systemd.user.services.roborev;
  settings = cfg.programs.roborev.settings;
}
