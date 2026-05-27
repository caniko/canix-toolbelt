{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.services.pypiServer;
in {
  options.canix-toolbelt.services.pypiServer = {
    enable = lib.mkEnableOption "PyPI server";

    pypi-packages-path = lib.mkOption {
      type = lib.types.path;
      description = "Path to the PyPI packages directory";
    };

    password-file = lib.mkOption {
      type = lib.types.path;
      description = "Path to the password file for PyPI server authentication";
    };

    port = lib.mkOption {
      type = lib.types.port;
      description = "Port to run the PyPI server on";
    };

    hostname = lib.mkOption {
      type = lib.types.str;
      description = "Hostname for the PyPI server";
    };

    routes = lib.mkOption {
      type = lib.types.listOf lib.types.attrs;
      readOnly = true;
      default = lib.optionals cfg.enable [
        {
          inherit (cfg) hostname port;
        }
      ];
      description = "Reverse-proxy route inputs exported by the PyPI server preset.";
    };
  };

  config = lib.mkIf cfg.enable {
    virtualisation.oci-containers.containers.pypiserver = {
      image = "pypiserver/pypiserver:latest";
      ports = ["127.0.0.1:${toString cfg.port}:8080"];
      volumes = [
        "${cfg.pypi-packages-path}:/data/packages"
        "/run/pypiserver/.htpasswd:/data/.htpasswd:ro"
      ];
      cmd = [
        "run"
        "-P"
        "/data/.htpasswd"
        "-a"
        "update,download"
        "/data/packages"
      ];
    };

    systemd.services.podman-pypiserver = {
      wants = ["network-online.target"];
      after = [
        "network-online.target"
        "nss-lookup.target"
      ];

      # Copy the secret to a location readable by the container.
      preStart = lib.mkAfter ''
        ${pkgs.coreutils}/bin/mkdir -p ${cfg.pypi-packages-path}
        ${pkgs.coreutils}/bin/mkdir -p /run/pypiserver
        ${pkgs.coreutils}/bin/cp ${cfg.password-file} /run/pypiserver/.htpasswd
        ${pkgs.coreutils}/bin/chmod 644 /run/pypiserver/.htpasswd
      '';
    };
  };
}
