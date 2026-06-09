{
  config,
  lib,
  pkgs,
  utils,
  ...
}:

let
  inherit (lib)
    foldlAttrs
    literalExpression
    mkDefault
    mkEnableOption
    mkIf
    mkOption
    mkPackageOption
    mkRemovedOptionModule
    mkRenamedOptionModule
    nameValuePair
    optionalAttrs
    optionals
    types
    ;

  cfg = config.services.forgejo-runner;
  settingsFormat = pkgs.formats.yaml { };
  secretsType =
    let
      pathType = types.pathWith {
        inStore = false;
        absolute = true;
      };
      base = types.oneOf [
        pathType
        (types.attrsOf base)
      ];
    in
    base
    // {
      description = "nested attribute set of ${pathType.description}";
    };

  hasDocker = config.virtualisation.docker.enable;
  hasPodman = config.virtualisation.podman.enable;
  hasContainerRuntime = hasDocker || hasPodman;
  labels =
    instance:
    instance.settings.runner.labels
    ++ (lib.flatten (
      lib.mapAttrsToList (_: value: value.labels or [ ]) instance.settings.server.connections
    ));
in
{
  meta.maintainers = pkgs.forgejo-runner.meta.maintainers;

  options.services.forgejo-runner = {
    package = mkPackageOption pkgs "forgejo-runner" { };

    instances = mkOption {
      default = { };
      description = ''
        Forgejo Runner instances.
      '';
      type = types.attrsOf (
        types.submodule (
          {
            options,
            config,
            name,
            ...
          }:
          {
            options.assertions = mkOption {
              type = types.listOf types.unspecified;
              default = [ ];
              internal = true;
              description = "NixOS assertion expressions for this instance.";
            };
            options.warnings = mkOption {
              type = types.listOf types.unspecified;
              default = [ ];
              internal = true;
              description = "NixOS warning expressions for this instance.";
            };
            config = {
              settings.runner.name = mkDefault name;
              assertions = [
                {
                  assertion = config.isDocker -> hasContainerRuntime;
                  message = ''
                    The instance `${name}' has at least one label of
                    type `:docker:' configured, but no compatible container runtime enabled.

                    You need to enable either
                    `config.virtualisation.docker.enable' or
                    `config.virtualisation.podman.enable'.
                  '';
                }
              ]
              ;
            };

            options = {
              enable = mkEnableOption "Forgejo Runner instance";

              settings = mkOption {
                default = { };
                description = ''
                  Free-form settings written directly to the `config.yaml` file.
                  Refer to [`config.example.yaml`] or run {command}`forgejo-runner generation-config` for supported values.

                  [`config.example.yaml`]: https://code.forgejo.org/forgejo/runner/src/branch/main/internal/pkg/config/config.example.yaml
                '';
                type = types.submodule {
                  freeformType = settingsFormat.type;
                  options = {
                    runner.labels = mkOption {
                      type = types.listOf types.str;
                      default = [ ];
                      description = ''
                        Labels used to map jobs to their runtime environment.

                        If you specify a label of type `:docker:`, the resulting runner service
                        will be automatically added to the Podman or Docker group.

                        See <https://forgejo.org/docs/latest/admin/actions/configuration/#choosing-labels>.
                      '';
                      example = literalExpression ''
                        [
                          "ubuntu-latest:docker://node:current"
                        ]
                      '';
                    };

                    server.connections = mkOption {
                      type = types.attrsOf (types.submodule (
                        { config, ... }:
                        {
                          freeformType = settingsFormat.type;
                          options = {
                            url = mkOption {
                              type = types.nullOr types.str;
                              default = null;
                            };
                            uuid = mkOption {
                              type = types.nullOr types.str;
                              default = null;
                            };
                            token = mkOption {
                              type = types.nullOr types.str;
                              default = null;
                            };
                            token_url = mkOption {
                              type = types.nullOr types.str;
                              default = null;
                            };
                            labels = mkOption {
                              type = types.listOf types.str;
                              default = [ ];
                            };
                          };
                        }
                      ));
                      default = { };
                      internal = true;
                      description = ''
                        Connection settings generated from {option}`connections`.
                      '';
                    };
                  };
                  config = lib.mapAttrsRecursive (
                    path: value: "file:$CREDENTIALS_DIRECTORY/${lib.join "__" path}"
                  ) config.secrets;
                };
              };

              connections = mkOption {
                type = types.attrsOf (types.submodule (
                  { name, ... }:
                  {
                    options = {
                      url = mkOption {
                        type = types.str;
                        example = "https://example.com/";
                        description = ''
                          Base URL of your Forgejo instance.
                        '';
                      };
                      uuid = mkOption {
                        type = types.str;
                        example = "c9e50be9-a7c3-4aee-ba35-624c4ff8c519";
                        description = ''
                          UUID of this runner.

                          See <https://forgejo.org/docs/latest/admin/actions/registration/>.
                        '';
                      };
                      token = mkOption {
                        type = types.nullOr types.str;
                        default = null;
                        example = "6634bb58be0db23cc013a2e72dd1828ae0257cf";
                        description = ''
                          Token of this runner.

                          See <https://forgejo.org/docs/latest/admin/actions/registration/>.
                        '';
                      };
                      tokenFile = mkOption {
                        type = types.nullOr types.path;
                        default = null;
                        description = ''
                          File containing the runner token.
                          The file content is read via systemd {manpage}`LoadCredential=` and referenced
                          as {option}`settings.server.connections.<name>.token_url`.

                          ::: {.note}
                          The file must contain the token on a single line, without any wrapper
                          such as `TOKEN=`.
                          :::
                        '';
                      };
                      labels = mkOption {
                        type = types.listOf types.str;
                        default = [ ];
                        description = ''
                          Instance-specific labels for this connection.

                          See <https://forgejo.org/docs/latest/admin/actions/configuration/#choosing-labels>.
                        '';
                      };
                      fetchInterval = mkOption {
                        type = types.nullOr types.str;
                        default = null;
                        example = "30s";
                        description = ''
                          How often this connection fetches new jobs from the Forgejo instance.
                        '';
                      };
                    };
                    config.settings.server.connections.${name} = {
                      inherit (config) url uuid;
                      token_url =
                        if config.tokenFile != null
                        then "file:$CREDENTIALS_DIRECTORY/${lib.escapeSystemdPath (builtins.toString config.tokenFile)}"
                        else null;
                      token =
                        if config.token != null
                        then config.token
                        else null;
                      labels = config.labels;
                    } // lib.optionalAttrs (config.fetchInterval != null) {
                      fetch_interval = config.fetchInterval;
                    };
                  }
                ));
                default = { };
                description = ''
                  One or more connections to Forgejo, each with a UUID and Token pair.

                  See <https://forgejo.org/docs/latest/admin/actions/registration/>.
                '';
                example = literalExpression ''
                  {
                    default = {
                      url = "https://example.com/";
                      uuid = "c9e50be9-a7c3-4aee-ba35-624c4ff8c519";
                      tokenFile = "/run/keys/forgejo-runner_token";
                    };
                  }
                '';
              };

              secrets = mkOption {
                type = secretsType;
                default = { };
                description = ''
                  This follows the same structure as {option}`settings`
                  but the value of each key is a path instead of a string, list or bool.

                  The specified secret path is then read by systemd via {manpage}`LoadCredential=`
                  and templated into {option}`settings` for you.
                '';
                example = literalExpression ''
                  {
                    server.connections.example = {
                      token_url = "/run/keys/forgejo-runner_token";
                    };
                    cache.secret_url = "/run/keys/forgejo-runner_cache-secret";
                  }
                '';
              };

              hostPackages = mkOption {
                type = types.listOf types.package;
                default = with pkgs; [
                  bash
                  coreutils
                  curl
                  gawk
                  gnused
                  nodejs
                  wget
                ];
                defaultText = literalExpression ''
                  with pkgs; [
                    bash coreutils curl gawk gnused nodejs wget
                  ]
                '';
                description = ''
                  List of packages available to workflows when the runner is
                  configured with a label of type `:host`.
                '';
              };

              isDocker = mkOption {
                internal = true;
                readOnly = true;
                type = types.bool;
                default = lib.any (label: lib.hasInfix ":docker:" label) (labels config);
                description = "Whether this instance has docker-type labels.";
              };

              isHost = mkOption {
                internal = true;
                readOnly = true;
                type = types.bool;
                default = lib.any (label: lib.hasSuffix ":host" label) (labels config);
                description = "Whether this instance has host-type labels.";
              };

              configFile = mkOption {
                internal = true;
                readOnly = true;
                type = types.path;
                default = settingsFormat.generate "config.yaml" (
                  lib.filterAttrsRecursive (n: _: n != "assertions" && n != "warnings") config.settings
                );
                description = "Generated config.yaml file for the runner daemon.";
              };
            };
          }
        )
      );
    };
  };

  config = mkIf (cfg.instances != { }) {
    assertions = foldlAttrs (assertions: _: instance: assertions ++ instance.assertions) [ ] cfg.instances;

    systemd.services =
      let
        mkRunnerService =
          name: instance:
          let
            wantsHost = instance.isHost;
            wantsDocker = instance.isDocker && hasDocker;
            wantsPodman = instance.isDocker && hasPodman;
          in
          nameValuePair "forgejo-runner-${utils.escapeSystemdPath name}" {
            inherit (instance) enable;
            description = "Forgejo Runner (${name})";
            wants = [ "network-online.target" ];
            after =
              [ "network-online.target" ]
              ++ optionals wantsDocker [ "docker.service" ]
              ++ optionals wantsPodman [ "podman.service" ];
            wantedBy = [ "multi-user.target" ];
            environment = {
              HOME = "/var/lib/forgejo-runner/${name}";
            } // optionalAttrs wantsPodman {
              DOCKER_HOST = "unix:///run/podman/podman.sock";
            };
            path = optionals wantsHost instance.hostPackages ++ [ pkgs.gitMinimal ];

            serviceConfig = {
              DynamicUser = true;
              StateDirectory = "forgejo-runner/${name}";
              WorkingDirectory = "-/var/lib/forgejo-runner/${name}";
              ExecPaths = optionals wantsHost [ "/var/lib/forgejo-runner/${name}" ];
              ExecStart = "${lib.getExe cfg.package} daemon --config ${instance.configFile}";
              Restart = "on-failure";
              RestartSec = 10;
              LoadCredential = lib.mapAttrsToListRecursive (
                path: value: "${lib.join "__" path}:${value}"
              ) instance.secrets;
              SupplementaryGroups =
                optionals wantsDocker [ "docker" ]
                ++ optionals wantsPodman [ "podman" ];
            };
          };
      in
      lib.mapAttrs' mkRunnerService (lib.filterAttrs (_: instance: instance.enable) cfg.instances);
  };
}
