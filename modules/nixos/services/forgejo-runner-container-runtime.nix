{
  config,
  lib,
  pkgs,
  utils,
  ...
}: let
  inherit (lib) concatStringsSep mkIf mkOption optionalAttrs types;
  inherit (utils) escapeSystemdPath;

  cfg = config.canix-toolbelt.services.forgejoRunner.containerRuntime;
  runnerCfg = config.services.forgejo.runner;

  actionRuntime = pkgs.buildEnv {
    name = "canix-forgejo-action-runtime";
    paths = cfg.actionRuntimePackages;
    pathsToLink = [
      "/bin"
      "/etc/ssl/certs"
    ];
  };

  actionRuntimeMounts =
    map
    (name: "-v ${actionRuntime}/bin/${name}:/usr/local/bin/${name}:ro")
    cfg.actionRuntimeExecutables;

  actionRuntimeVolumeSources =
    [
      "${actionRuntime}"
      "${actionRuntime}/etc/ssl/certs"
    ]
    ++ map (name: "${actionRuntime}/bin/${name}") cfg.actionRuntimeExecutables;

  runnerImage = pkgs.dockerTools.buildLayeredImage {
    name = cfg.imageName;
    tag = cfg.imageTag;
    contents = cfg.imageContents ++ cfg.imageExtraContents;
    config = {
      Env = [
        "PATH=/bin:/usr/bin"
        "SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt"
        "NIX_SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt"
      ];
      WorkingDir = "/";
      Cmd = ["/bin/bash"];
    };
  };

  runtimeOptions =
    [
      "-v /nix/store:/nix/store:ro"
      "-v ${actionRuntime}/etc/ssl/certs:/canix-forgejo-action-certs:ro"
      "-e SSL_CERT_FILE=/canix-forgejo-action-certs/ca-bundle.crt"
      "-e NIX_SSL_CERT_FILE=/canix-forgejo-action-certs/ca-bundle.crt"
    ]
    ++ actionRuntimeMounts
    ++ cfg.extraContainerOptions;

  runnerServiceNames =
    map
    (instance: "forgejo-runner@${escapeSystemdPath instance.name}.service")
    (lib.attrValues runnerCfg.instances);
in {
  options.canix-toolbelt.services.forgejoRunner.containerRuntime = {
    enable = lib.mkEnableOption "reusable Forgejo runner container runtime";

    imageName = mkOption {
      type = types.str;
      default = "canix-runner";
      description = "Local runner OCI image name loaded into the container runtime.";
    };

    imageTag = mkOption {
      type = types.str;
      default = "local";
      description = "Local runner OCI image tag.";
    };

    imageContents = mkOption {
      type = types.listOf types.package;
      default = with pkgs; [
        dockerTools.caCertificates
        bashInteractive
        coreutils
        gitMinimal
        nodejs_24
        curl
        gnutar
        gzip
        which
      ];
      description = "Packages baked into the base runner image.";
    };

    imageExtraContents = mkOption {
      type = types.listOf types.package;
      default = [];
      description = "Additional packages to bake into the base runner image.";
    };

    actionRuntimePackages = mkOption {
      type = types.listOf types.package;
      default = with pkgs; [
        bashInteractive
        coreutils
        curl
        dockerTools.caCertificates
        gawk
        gitMinimal
        gnused
        gnutar
        gzip
        nodejs_24
        which
      ];
      description = "Packages exposed to workflow job containers for JavaScript actions.";
    };

    actionRuntimeExecutables = mkOption {
      type = types.listOf types.str;
      default = [
        "bash"
        "curl"
        "git"
        "gzip"
        "node"
        "tar"
        "which"
      ];
      description = "Action-runtime executables mounted into job containers under /usr/local/bin.";
    };

    extraContainerOptions = mkOption {
      type = types.listOf types.str;
      default = [];
      description = "Additional act container options appended after the reusable runtime mounts.";
    };

    extraValidVolumes = mkOption {
      type = types.listOf types.str;
      default = [];
      description = "Additional volume sources allowed for workflow job containers.";
    };

    imageRef = mkOption {
      type = types.str;
      readOnly = true;
      description = "Fully qualified local image reference for Forgejo runner labels.";
    };

    containerOptions = mkOption {
      type = types.str;
      readOnly = true;
      description = "Reusable container options for Forgejo runner job containers.";
    };

    validVolumes = mkOption {
      type = types.listOf types.str;
      readOnly = true;
      description = "Reusable valid volume sources for Forgejo runner job containers.";
    };
  };

  config = mkIf cfg.enable {
    canix-toolbelt.services.forgejoRunner.containerRuntime = {
      imageRef = "docker://localhost/${cfg.imageName}:${cfg.imageTag}";
      containerOptions = concatStringsSep " " runtimeOptions;
      validVolumes =
        [
          "/nix/store"
        ]
        ++ actionRuntimeVolumeSources
        ++ cfg.extraValidVolumes;
    };

    systemd.services.forgejo-runner-image-load =
      {
        description = "Load canix forgejo-runner OCI image into podman";
        wantedBy = ["multi-user.target"];
        restartTriggers = [runnerImage];
        serviceConfig = {
          Type = "oneshot";
          RemainAfterExit = true;
          ExecStart = "${pkgs.writeShellScript "forgejo-runner-image-load" ''
            set -eu
            ${config.virtualisation.podman.package}/bin/podman load -i ${runnerImage}
          ''}";
        };
      }
      // optionalAttrs config.virtualisation.podman.enable {
        requires = ["podman.service"];
        after = ["podman.service"];
        before = runnerServiceNames;
      }
      // optionalAttrs config.virtualisation.docker.enable {
        requires = ["docker.service"];
        after = ["docker.service"];
        before = runnerServiceNames;
      };
  };
}
