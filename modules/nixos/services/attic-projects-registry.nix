# Auto-generates agenix wiring from an attic projects JSON registry.
#
# A project entry of the shape:
#
#   "<name>" = { consumers = [ "atlas" ]; runner = "atlas"; }
#
# becomes `age.secrets.attic-<name>-token` on every host listed in
# `consumers`. On the host named by `runner`, a stable tokens directory is
# materialised for forgejo-runner containers to bind-mount.
{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.atticProjects;
  registry = lib.importJSON cfg.registry;
  inherit (config.networking) hostName;

  forThisHost =
    lib.filterAttrs
    (_: meta: lib.elem hostName (meta.consumers or []))
    registry;

  runnerProjects =
    lib.filterAttrs
    (_: meta: (meta.runner or null) == hostName)
    registry;

  isRunnerHost = runnerProjects != {};
  tokensDir = "/run/canix-attic-tokens";
in {
  options.canix-toolbelt.atticProjects = {
    registry = lib.mkOption {
      type = lib.types.path;
      description = "Path to the attic-projects JSON registry.";
    };

    secretPath = lib.mkOption {
      type = lib.types.functionTo lib.types.path;
      description = "Function mapping a project name to its agenix source path.";
    };

    tokensDir = lib.mkOption {
      type = lib.types.str;
      default = tokensDir;
      readOnly = true;
      description = ''
        Read-only directory exposed on a runner host containing one file per
        registered project (`<name>` -> token plaintext). Forgejo-runner
        container instances should bind-mount this into jobs as
        `$ATTIC_TOKENS_DIR`.
      '';
    };

    tokensDirOwner = lib.mkOption {
      type = lib.types.str;
      default = "root";
      description = "Owner for the runner token directory.";
    };

    tokensDirGroup = lib.mkOption {
      type = lib.types.str;
      default = "root";
      description = "Group for the runner token directory.";
    };

    tokenFileMode = lib.mkOption {
      type = lib.types.str;
      default = "0444";
      description = "Mode applied to runner-host token files.";
    };
  };

  config = {
    age.secrets =
      lib.mapAttrs'
      (name: meta: let
        isRunnerToken = (meta.runner or null) == hostName;
      in
        lib.nameValuePair "attic-${name}-token" ({
            rekeyFile = cfg.secretPath name;
          }
          // lib.optionalAttrs isRunnerToken {
            mode = cfg.tokenFileMode;
          }))
      forThisHost;

    systemd.tmpfiles.settings."10-canix-attic-tokens" = lib.mkIf isRunnerHost (
      {
        ${cfg.tokensDir}.d = {
          mode = "0755";
          user = cfg.tokensDirOwner;
          group = cfg.tokensDirGroup;
        };
      }
      // lib.mapAttrs' (
        name: _:
          lib.nameValuePair "${cfg.tokensDir}/${name}" {
            "L+".argument = config.age.secrets."attic-${name}-token".path;
          }
      )
      runnerProjects
    );
  };
}
