{
  pkgs,
  homeManager,
}: let
  inherit (pkgs) lib;
  eval = extra:
    (homeManager.lib.homeManagerConfiguration {
      inherit pkgs;
      modules = [
        ../../modules/home/roborev
        {
          home.username = "roborev-fixture";
          home.homeDirectory = "/home/roborev-fixture";
          home.stateVersion = "26.05";
        }
        extra
      ];
    }).config;
  enabled = {
    programs.roborev = {
      enable = true;
      package = pkgs.hello;
      agentCommands.opencode = "${pkgs.coreutils}/bin/true";
    };
  };
  program = eval enabled;
  service = eval (lib.recursiveUpdate enabled {
    services.roborev.enable = true;
  });
  changed = eval (lib.recursiveUpdate enabled {
    services.roborev.enable = true;
    programs.roborev = {
      settings.max_workers = 3;
      agentCommands.opencode = "${pkgs.git}/bin/git";
    };
  });
  disabled = eval {};
  inactiveWorker = eval {
    programs.roborev = {
      enable = lib.mkDefault false;
      package = pkgs.hello;
      agentCommands.opencode = "/fixture/approved-opencode";
      settings.ci = {
        enabled = false;
        repos = [];
        github_app_private_key = "/run/fixture/app-key.pem";
      };
    };
    services.roborev = {
      enable = lib.mkDefault false;
      requiredFiles = ["/run/fixture/app-key.pem"];
    };
  };
  implied = eval {
    programs.roborev.package = pkgs.hello;
    programs.roborev.agentCommands.opencode = "${pkgs.coreutils}/bin/true";
    services.roborev.enable = true;
  };
  reject = extra: let
    cfg = eval (lib.recursiveUpdate enabled extra);
    result =
      builtins.tryEval (builtins.deepSeq cfg.assertions
        (builtins.any (a: !a.assertion) cfg.assertions));
  in
    !result.success || result.value;
  accepted = cfg: builtins.all (a: a.assertion) cfg.assertions;
  merged = eval {
    imports = [
      enabled
      {
        programs.roborev.settings.max_workers = 3;
        programs.roborev.settings.ci.poll_interval = "5m";
      }
    ];
  };
  mappings = eval (lib.recursiveUpdate enabled {
    programs.roborev.agentCommands = {
      codex = "/fixture/codex with spaces";
      claude-code = "/fixture/claude-code";
    };
  });
  integrated =
    (import (pkgs.path + "/nixos/lib/eval-config.nix") {
      inherit pkgs;
      system = pkgs.stdenv.hostPlatform.system;
      modules = [
        homeManager.nixosModules.home-manager
        {
          system.stateVersion = "26.05";
          users.users.roborev-fixture = {
            isNormalUser = true;
            home = "/home/roborev-fixture";
          };
          home-manager.useGlobalPkgs = true;
          home-manager.users.roborev-fixture = {
            imports = [../../modules/home/roborev enabled];
            home.stateVersion = "26.05";
            services.roborev.enable = true;
          };
        }
      ];
    }).config.home-manager.users.roborev-fixture;
  darwin =
    (homeManager.lib.homeManagerConfiguration {
      pkgs = import pkgs.path {system = "aarch64-darwin";};
      modules = [
        ../../modules/home/roborev
        {
          home.username = "roborev-fixture";
          home.homeDirectory = "/Users/roborev-fixture";
          home.stateVersion = "26.05";
          programs.roborev = {
            package = pkgs.hello;
            agentCommands.opencode = "/fixture/opencode";
          };
          services.roborev.enable = true;
        }
      ];
    }).config;
in {
  E01_disabled =
    !(disabled.systemd.user.services ? roborev)
    && !(disabled.home.file ? roborev-config)
    && !(disabled.home.activation ? roborevDataDir)
    && !(disabled.home.activation ? roborevDataDirCheck);
  E02_program =
    accepted program
    && !(program.systemd.user.services ? roborev)
    && program.home.file.roborev-config.target == ".roborev/config.toml";
  E02_service = accepted service && service.systemd.user.services.roborev.Service.Type == "notify";
  E02_implied_program = accepted implied && implied.programs.roborev.enable;
  E02_conflict = reject {
    programs.roborev.enable = false;
    services.roborev.enable = true;
  };
  E03_no_user_manager = reject {
    services.roborev.enable = true;
    systemd.user.enable = false;
  };
  E03_integrated =
    accepted integrated
    && integrated.programs.roborev.settings == service.programs.roborev.settings
    && integrated.systemd.user.services.roborev.Service.ExecStart == service.systemd.user.services.roborev.Service.ExecStart
    && integrated.home.file.roborev-config.target == service.home.file.roborev-config.target;
  # Home Manager forces failed assertions before returning its public config.
  E03_darwin_rejected = !(builtins.tryEval (builtins.deepSeq darwin.assertions true)).success;
  E04_merge =
    accepted merged
    && merged.programs.roborev.settings.max_workers == 3
    && merged.programs.roborev.settings.ci.enabled == false;
  E04_reserved = reject {programs.roborev.settings.default_agent = "codex";};
  E05_mappings = accepted mappings;
  E05_missing = reject {programs.roborev.defaultAgent = "codex";};
  E05_unknown = reject {programs.roborev.agentCommands.unknown = "/fixture/no";};
  E05_control = reject {programs.roborev.agentCommands.opencode = "/fixture/no\ninjected";};
  E06_inline_auth = reject {programs.roborev.settings.auth_key = "SYNTHETIC_SENTINEL";};
  E06_inline_web = reject {programs.roborev.settings.web.auth_token = "SYNTHETIC_SENTINEL";};
  E06_inline_anthropic = reject {programs.roborev.settings.anthropic_api_key = "SYNTHETIC_SENTINEL";};
  # Construct the synthetic PEM header at evaluation time for the source scanner.
  E06_inline_app = reject {programs.roborev.settings.ci.github_app_private_key = builtins.concatStringsSep " " ["-----BEGIN" "PRIVATE KEY-----"];};
  E06_store_runtime = reject {
    services.roborev = {
      enable = true;
      requiredFiles = ["/nix/store/secret"];
    };
  };
  E06_store_traversal = reject {
    services.roborev = {
      enable = true;
      requiredFiles = ["/run/../nix/store/secret"];
    };
  };
  E06_store_web = reject {programs.roborev.settings.web.auth_token_file = "/nix/store/secret";};
  E06_env_reference = accepted (eval (lib.recursiveUpdate enabled {
    programs.roborev.settings.anthropic_api_key = "\${SYNTHETIC_API_KEY}";
    programs.roborev.settings.ci.github_app_private_key = "/run/fixture-app-key.pem";
  }));
  E06_path_runtime = reject {
    services.roborev = {
      enable = true;
      environmentFiles = [./eval.nix];
    };
  };
  E06_unit_injection = reject {
    services.roborev = {
      enable = true;
      environmentFiles = ["/run/secret\nExecStart=evil"];
    };
  };
  E06_environment_file_continuation = reject {
    services.roborev = {
      enable = true;
      environmentFiles = ["/run/file\\"];
    };
  };
  E06_environment_file_trailing_space = reject {
    services.roborev = {
      enable = true;
      environmentFiles = ["/run/file "];
    };
  };
  E06_environment_file_glob = reject {
    services.roborev = {
      enable = true;
      environmentFiles = ["/run/file*"];
    };
  };
  E08_defaults =
    program.programs.roborev.settings.server_addr
    == "unix://"
    && program.programs.roborev.settings.max_workers == 2
    && program.programs.roborev.settings.isolate_reviews
    && !program.programs.roborev.settings.ci.enabled
    && !program.programs.roborev.settings.web.enabled
    && !program.programs.roborev.enableTelemetry;
  E09_data_outside_home = reject {programs.roborev.dataDir = "/run/roborev";};
  E09_inactive_worker =
    accepted inactiveWorker
    && !inactiveWorker.programs.roborev.enable
    && !inactiveWorker.services.roborev.enable
    && !inactiveWorker.programs.roborev.settings.ci.enabled
    && inactiveWorker.programs.roborev.settings.ci.repos == []
    && !(inactiveWorker.systemd.user.services ? roborev)
    && !(inactiveWorker.home.file ? roborev-config);
  E09_data_dotdot = reject {programs.roborev.dataDir = "/home/roborev-fixture/../other";};
  E09_data_trailing = reject {programs.roborev.dataDir = "/home/roborev-fixture/.roborev/";};
  E10_restart =
    builtins.elem service.programs.roborev.finalPackage
    service.systemd.user.services.roborev.Unit.X-Restart-Triggers
    && builtins.elem service.programs.roborev._configFile
    service.systemd.user.services.roborev.Unit.X-Restart-Triggers;
  S06_restart_identity =
    changed.programs.roborev._configFile.drvPath
    != service.programs.roborev._configFile.drvPath
    && changed.programs.roborev.finalPackage.drvPath != service.programs.roborev.finalPackage.drvPath
    && changed.systemd.user.services.roborev.Unit.X-Restart-Triggers != service.systemd.user.services.roborev.Unit.X-Restart-Triggers;
}
