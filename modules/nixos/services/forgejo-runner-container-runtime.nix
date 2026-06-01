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

  jobContainerPath = concatStringsSep ":" [
    "/usr/local/bin"
    "/usr/local/cargo/bin"
    "/root/.cargo/bin"
    "/root/.nix-profile/bin"
    "/nix/var/nix/profiles/default/bin"
    "/nix/var/nix/profiles/default/sbin"
    "/bin"
    "/usr/bin"
    "/sbin"
    "/usr/sbin"
  ];

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

  sudoShim = pkgs.writeShellScriptBin "sudo" ''
    set -eu

    while [ "$#" -gt 0 ]; do
      case "$1" in
        -E|-H|-n|-S|--preserve-env|--preserve-env=*)
          shift
          ;;
        --)
          shift
          break
          ;;
        -u|-g)
          shift
          if [ "$#" -gt 0 ]; then
            shift
          fi
          ;;
        *)
          break
          ;;
      esac
    done

    exec "$@"
  '';

  runnerImage = pkgs.dockerTools.buildLayeredImage {
    name = cfg.imageName;
    tag = cfg.imageTag;
    contents = cfg.imageContents ++ cfg.imageExtraContents;
    extraCommands = ''
      mkdir -p etc root usr/bin
      cat > etc/passwd <<'EOF'
      root:x:0:0:root:/root:/bin/bash
      EOF
      cat > etc/group <<'EOF'
      root:x:0:
      EOF
      ln -s /bin/env usr/bin/env
    '';
    config = {
      Env = [
        "PATH=/bin:/usr/bin"
        "USER=root"
        "HOME=/root"
        "SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt"
        "NIX_SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt"
      ];
      WorkingDir = "/";
      Cmd = ["/bin/bash"];
    };
  };

  hostNixRunnerImage = pkgs.dockerTools.buildLayeredImage {
    name = cfg.hostNixImageName;
    tag = cfg.hostNixImageTag;
    contents = cfg.hostNixImageContents ++ cfg.hostNixImageExtraContents;
    extraCommands = ''
      mkdir -p etc root usr/bin
      cat > etc/passwd <<'EOF'
      root:x:0:0:root:/root:/bin/bash
      EOF
      cat > etc/group <<'EOF'
      root:x:0:
      EOF
      ln -s /bin/env usr/bin/env
    '';
    config = {
      Env = [
        "PATH=/bin:/usr/bin"
        "NIX_REMOTE=daemon"
        "NIX_PAGER=cat"
        "USER=root"
        "HOME=/root"
        "SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt"
        "NIX_SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt"
      ];
      WorkingDir = "/";
      Cmd = ["/bin/bash"];
    };
  };

  localRunnerImage = "localhost/${cfg.imageName}:${cfg.imageTag}";
  localHostNixRunnerImage = "localhost/${cfg.hostNixImageName}:${cfg.hostNixImageTag}";

  runtimeOptions =
    [
      "-e PATH=${jobContainerPath}"
      "-v /nix/store:/nix/store:ro"
      "-v ${actionRuntime}/etc/ssl/certs:/canix-forgejo-action-certs:ro"
    ]
    ++ actionRuntimeMounts
    ++ cfg.extraContainerOptions
    ++ [
      "-e SSL_CERT_FILE=/canix-forgejo-action-certs/ca-bundle.crt"
      "-e NIX_SSL_CERT_FILE=/canix-forgejo-action-certs/ca-bundle.crt"
    ];

  hostNixRuntimeOptions =
    [
      "-e PATH=${jobContainerPath}"
      "-e NIX_REMOTE=daemon"
      "-e NIX_PAGER=cat"
      "-e USER=root"
      "-v /nix/store:/nix/store:ro"
      "-v /nix/var/nix/daemon-socket/socket:/nix/var/nix/daemon-socket/socket"
      "-v ${actionRuntime}/etc/ssl/certs:/canix-forgejo-action-certs:ro"
    ]
    ++ actionRuntimeMounts
    ++ cfg.hostNixExtraContainerOptions
    ++ [
      "-e SSL_CERT_FILE=/canix-forgejo-action-certs/ca-bundle.crt"
      "-e NIX_SSL_CERT_FILE=/canix-forgejo-action-certs/ca-bundle.crt"
    ];

  runnerUnitNames =
    map
    (instance: "forgejo-runner@${escapeSystemdPath instance.name}")
    (lib.attrValues runnerCfg.instances);

  runnerServiceNames = map (name: "${name}.service") runnerUnitNames;

  ensureRunnerImages = pkgs.writeShellScript "forgejo-runner-image-load" ''
    set -eu

    podman=${lib.escapeShellArg "${config.virtualisation.podman.package}/bin/podman"}

    ensure_image() {
      image="$1"
      archive="$2"
      marker_dir="''${STATE_DIRECTORY:-/var/lib/forgejo-runner-image-load}"
      marker="$marker_dir/$(printf '%s' "$image" | tr '/:' '__').archive"

      mkdir -p "$marker_dir"

      if "$podman" image exists "$image" && [ -f "$marker" ] && [ "$(cat "$marker")" = "$archive" ]; then
        echo "Forgejo runner image already present: $image"
      else
        echo "Loading Forgejo runner image: $image"
        "$podman" load -i "$archive"
      fi

      if ! "$podman" image exists "$image"; then
        echo "Forgejo runner image '$image' is still missing after loading '$archive'" >&2
        exit 1
      fi

      printf '%s\n' "$archive" > "$marker.tmp"
      mv "$marker.tmp" "$marker"
    }

    ensure_image ${lib.escapeShellArg localRunnerImage} ${lib.escapeShellArg "${runnerImage}"}
    ensure_image ${lib.escapeShellArg localHostNixRunnerImage} ${lib.escapeShellArg "${hostNixRunnerImage}"}
  '';
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

    hostNixImageName = mkOption {
      type = types.str;
      default = "canix-nix-runner";
      description = "Local host-nix runner OCI image name loaded into the container runtime.";
    };

    hostNixImageTag = mkOption {
      type = types.str;
      default = "local";
      description = "Local host-nix runner OCI image tag.";
    };

    imageContents = mkOption {
      type = types.listOf types.package;
      default = with pkgs; [
        dockerTools.caCertificates
        sudoShim
        bashInteractive
        coreutils
        gitMinimal
        gawk
        gnugrep
        gnused
        nodejs_24
        curl
        gnutar
        gzip
        which
        xz
      ];
      description = "Packages baked into the base runner image.";
    };

    imageExtraContents = mkOption {
      type = types.listOf types.package;
      default = [];
      description = "Additional packages to bake into the base runner image.";
    };

    hostNixImageContents = mkOption {
      type = types.listOf types.package;
      default = with pkgs; [
        dockerTools.caCertificates
        sudoShim
        nix
        bashInteractive
        coreutils
        gitMinimal
        gawk
        gnugrep
        gnupg
        gnused
        nodejs_24
        curl
        gnutar
        gzip
        which
        xz
      ];
      description = "Packages baked into the trusted host-nix runner image.";
    };

    hostNixImageExtraContents = mkOption {
      type = types.listOf types.package;
      default = [];
      description = "Additional packages to bake into the trusted host-nix runner image.";
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
        gnugrep
        gnused
        gnutar
        gzip
        nodejs_24
        which
        xz
      ];
      description = "Packages exposed to workflow job containers for JavaScript actions.";
    };

    actionRuntimeExecutables = mkOption {
      type = types.listOf types.str;
      default = [
        "awk"
        "bash"
        "curl"
        "env"
        "git"
        "grep"
        "gzip"
        "node"
        "sed"
        "sh"
        "tar"
        "tail"
        "which"
        "xz"
      ];
      description = "Action-runtime executables mounted into job containers under /usr/local/bin.";
    };

    extraContainerOptions = mkOption {
      type = types.listOf types.str;
      default = [];
      description = "Additional act container options appended after the reusable runtime mounts.";
    };

    hostNixExtraContainerOptions = mkOption {
      type = types.listOf types.str;
      default = [];
      description = "Additional act container options appended after the trusted host-nix runtime mounts.";
    };

    extraValidVolumes = mkOption {
      type = types.listOf types.str;
      default = [];
      description = "Additional volume sources allowed for workflow job containers.";
    };

    hostNixExtraValidVolumes = mkOption {
      type = types.listOf types.str;
      default = [];
      description = "Additional volume sources allowed for trusted host-nix workflow job containers.";
    };

    imageRef = mkOption {
      type = types.str;
      readOnly = true;
      description = "Fully qualified local image reference for Forgejo runner labels.";
    };

    hostNixImageRef = mkOption {
      type = types.str;
      readOnly = true;
      description = "Fully qualified local trusted host-nix image reference for Forgejo runner labels.";
    };

    containerOptions = mkOption {
      type = types.str;
      readOnly = true;
      description = "Reusable container options for Forgejo runner job containers.";
    };

    hostNixContainerOptions = mkOption {
      type = types.str;
      readOnly = true;
      description = "Reusable container options for trusted host-nix Forgejo runner job containers.";
    };

    validVolumes = mkOption {
      type = types.listOf types.str;
      readOnly = true;
      description = "Reusable valid volume sources for Forgejo runner job containers.";
    };

    hostNixValidVolumes = mkOption {
      type = types.listOf types.str;
      readOnly = true;
      description = "Reusable valid volume sources for trusted host-nix Forgejo runner job containers.";
    };
  };

  config = mkIf cfg.enable {
    canix-toolbelt.services.forgejoRunner.containerRuntime = {
      imageRef = "docker://${localRunnerImage}";
      hostNixImageRef = "docker://${localHostNixRunnerImage}";
      containerOptions = concatStringsSep " " runtimeOptions;
      hostNixContainerOptions = concatStringsSep " " hostNixRuntimeOptions;
      validVolumes =
        [
          "/nix/store"
        ]
        ++ actionRuntimeVolumeSources
        ++ cfg.extraValidVolumes;
      hostNixValidVolumes =
        [
          "/nix/store"
          "/nix/var/nix/daemon-socket/socket"
        ]
        ++ actionRuntimeVolumeSources
        ++ cfg.hostNixExtraValidVolumes;
    };

    systemd.services =
      {
        forgejo-runner-image-load =
          {
            description = "Ensure canix forgejo-runner OCI images exist in podman";
            wantedBy = ["multi-user.target"];
            restartTriggers = [runnerImage hostNixRunnerImage];
            serviceConfig = {
              Type = "oneshot";
              RemainAfterExit = false;
              StateDirectory = "forgejo-runner-image-load";
              ExecStart = "${ensureRunnerImages}";
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
      }
      // lib.genAttrs runnerUnitNames (_: {
        requires = ["forgejo-runner-image-load.service"];
        after = ["forgejo-runner-image-load.service"];
      });

    systemd.timers.forgejo-runner-image-load = {
      description = "Periodically ensure canix forgejo-runner OCI images exist";
      wantedBy = ["timers.target"];
      timerConfig = {
        OnBootSec = "2min";
        OnUnitActiveSec = "5min";
        Unit = "forgejo-runner-image-load.service";
      };
    };
  };
}
