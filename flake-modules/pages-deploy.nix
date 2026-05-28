# Thin flake-parts wrapper around lib.mkDeployPagesApp.
{
  flake-parts-lib,
  lib,
  ...
}: {
  options.perSystem = flake-parts-lib.mkPerSystemOption ({
    config,
    pkgs,
    ...
  }: let
    cfg = config.canix-toolbelt.pages-deploy;
    toolbeltLib = import ../lib {inherit lib;};
  in {
    options.canix-toolbelt.pages-deploy = {
      enable = lib.mkEnableOption "Codeberg Pages deploy app";

      sitePackage = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = null;
        description = "Static site package to publish to the Pages branch.";
      };

      appName = lib.mkOption {
        type = lib.types.str;
        default = "deploy-pages";
        description = "Attribute name under `apps`.";
      };

      remoteEnvVar = lib.mkOption {
        type = lib.types.str;
        default = "DEPLOY_REMOTE";
        description = "Environment variable containing the git remote name to push.";
      };

      branch = lib.mkOption {
        type = lib.types.str;
        default = "pages";
        description = "Pages branch to update.";
      };

      commitMessageTemplate = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Optional commit message template; `{timestamp}` expands to UTC ISO-like time.";
      };
    };

    config = lib.mkIf cfg.enable {
      assertions = [
        {
          assertion = cfg.sitePackage != null;
          message = "canix-toolbelt.pages-deploy.sitePackage must be set when pages-deploy is enabled";
        }
      ];

      apps.${cfg.appName} = {
        type = "app";
        program = "${
          toolbeltLib.mkDeployPagesApp {
            inherit pkgs;
            inherit (cfg) sitePackage;
            name = cfg.appName;
            inherit (cfg) remoteEnvVar branch commitMessageTemplate;
          }
        }/bin/${cfg.appName}";
      };
    };
  });
}
