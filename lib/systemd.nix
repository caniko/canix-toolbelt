# systemd — helpers for common systemd unit patterns.
#
# Three functions:
#
#   mkDownloadOneshot  — download a URL to a file if missing, retrying on failure
#   mkPathTrigger      — systemd.path unit that triggers a service when a file appears
#   mkConditionalOneshot — oneshot whose execution is gated on a file condition
#
{lib}: {
  # mkDownloadOneshot :: { pkgs, name, url, path, authTokenFile?, extraScript?,
  #                        restartSec?, stateDirMode? } -> { <name> = <service def> }
  #
  # Returns a systemd.service definition for a oneshot that downloads `url`
  # to `path`.  Uses ConditionPathExists=!${path} so systemd skips the unit
  # entirely when the file already exists (no bash file-existence guard
  # needed).  Restart=on-failure + StartLimitIntervalSec=0 makes systemd
  # retry the download forever without a bash loop.  After=network-online.target
  # nss-lookup.target ensures DNS is available.
  #
  # Example:
  #   systemd.services = mkDownloadOneshot {
  #     inherit pkgs;
  #     name = "bonsai-model";
  #     url  = "https://huggingface.co/…/model.gguf";
  #     path = "/var/lib/models/bonsai/model.gguf";
  #   };
  mkDownloadOneshot = {
    pkgs,
    name,
    url,
    path,
    authTokenFile ? null,
    extraScript ? "",
    restartSec ? 30,
  }: let
    serviceName = "download-${name}";
    parentDir = builtins.dirOf path;
  in {
    ${serviceName} = {
      description = "Download ${name}";
      after = ["network-online.target" "nss-lookup.target"];
      wants = ["network-online.target"];
      path = [pkgs.curl];

      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        Restart = "on-failure";
        RestartSec = toString restartSec;
        StartLimitIntervalSec = "0";
        ConditionPathExists = "!${path}";
      };

      script = let
        tokenFlag =
          lib.optionalString (authTokenFile != null)
          "-H \"Authorization: Bearer $(cat ${lib.escapeShellArg authTokenFile})\"";
      in ''
        set -euo pipefail
        mkdir -p ${lib.escapeShellArg parentDir}
        tmp="${path}.tmp"
        ${pkgs.curl}/bin/curl -L --fail --progress-bar \
          ${tokenFlag} \
          -o "$tmp" \
          ${lib.escapeShellArg url}
        mv "$tmp" "${path}"
        chmod 644 "${path}"
        ${extraScript}
      '';
    };
  };

  # mkPathTrigger :: { name, path, triggersService? } -> { <name> = <path unit def> }
  #
  # Returns a systemd.path definition that watches `path` and activates the
  # unit named by `triggersService` when the file appears.  The path unit
  # is named `<name>` by default; if `triggersService` is omitted systemd
  # pairs it with a `<name>.service` unit automatically.
  #
  # Example:
  #   systemd.paths = mkPathTrigger {
  #     name = "bonsai-model";
  #     path = "/var/lib/models/bonsai/model.gguf";
  #     triggersService = "brainrouter.service";
  #   };
  mkPathTrigger = {
    name,
    path,
    triggersService ? null,
  }: let
    pathUnitName = name;
  in {
    ${pathUnitName} = {
      wantedBy = ["multi-user.target"];
      pathConfig =
        {
          PathExists = path;
          # Unit= goes in the [Path] section for path units, not [Unit].
          # systemd reads this to know which service to trigger.
        }
        // lib.optionalAttrs (triggersService != null) {
          Unit = triggersService;
        };
    };
  };

  # mkConditionalOneshot :: { name, path, conditionExists, onEnter, onExit?,
  #                          script, serviceConfig? } -> { <name> = <service def> }
  #
  # Returns a systemd.service definition for a oneshot gated by a file
  # condition.  When `conditionExists` is true the service runs only if
  # `path` exists; when false the service runs only if `path` does NOT
  # exist.  Replaces the common `if [ -f "$file" ]` / `if [ ! -f "$file" ]`
  # patterns that appear in dozens of preStart/script blocks.
  #
  # Example:
  #   systemd.services = mkConditionalOneshot {
  #     name = "generate-postgres-tls-cert";
  #     path = "/var/lib/postgresql/server.key";
  #     conditionExists = false;
  #     script = ''
  #       openssl req -new -x509 -nodes -out server.crt -keyout server.key ...
  #     '';
  #   };
  mkConditionalOneshot = {
    name,
    path,
    conditionExists,
    script,
    extraServiceConfig ? {},
  }:
    assert builtins.isBool conditionExists; {
      ${name} = {
        description = "${name} (conditional${
          if conditionExists
          then " — run when exists"
          else " — skip when exists"
        })";
        serviceConfig =
          {
            Type = "oneshot";
            RemainAfterExit = true;
            ConditionPathExists =
              if conditionExists
              then path
              else "!${path}";
          }
          // extraServiceConfig;

        inherit script;
      };
    };
}
