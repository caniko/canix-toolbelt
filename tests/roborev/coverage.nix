{
  pkgs,
  eval,
  enabled,
  reject,
  accepted,
}: let
  inherit (pkgs) lib;
  configure = extra: eval (lib.recursiveUpdate enabled extra);
  agents = ["codex" "claude-code" "gemini" "copilot" "cursor" "opencode" "droid" "kilo" "kiro" "pi" "grok"];
  runtime = eval {
    programs.roborev = {
      enable = true;
      package = pkgs.hello;
      runtimeConfigFile = "/run/user/1000/roborev-config.toml";
    };
  };
  acp = configure {
    programs.roborev = {
      defaultAgent = "acp.reviewer";
      settings.acp.reviewer = {
        command = "/fixture/acp";
        args = ["--stdio"];
        read_only_mode = "plan";
        model = "fixture-model";
      };
    };
  };
  repositories = configure {
    programs.roborev.repositories.example.settings = {
      excluded_branches = ["scratch"];
      post_commit_batch_size = 5;
      auto_design_review.enabled = false;
      ci.reviews = {};
    };
  };
in {
  C01_all_review_agents = builtins.all (name:
    accepted (configure {
      programs.roborev.defaultAgent = name;
      programs.roborev.agentCommands.${name} = "/fixture/${name}";
    }))
  agents;
  C02_existing_command_setting = accepted (configure {
    programs.roborev.defaultAgent = "gemini";
    programs.roborev.settings.gemini_cmd = "/fixture/gemini";
  });
  C03_equal_command_definitions = accepted (configure {
    programs.roborev.agentCommands.gemini = "/fixture/gemini";
    programs.roborev.settings.gemini_cmd = "/fixture/gemini";
  });
  C04_conflicting_command_definitions = reject {
    programs.roborev.agentCommands.gemini = "/fixture/gemini";
    programs.roborev.settings.gemini_cmd = "/fixture/other";
  };
  C05_named_acp = accepted acp;
  C06_undefined_acp = reject {programs.roborev.defaultAgent = "acp.missing";};
  C07_embedding_env = accepted (configure {
    programs.roborev.settings.search.embeddings.api_key.env = "FIXTURE_KEY";
  });
  C08_embedding_file = accepted (configure {
    programs.roborev.settings.search.embeddings.api_key.file = "/run/user/1000/embedding-key";
  });
  C09_embedding_store = reject {
    programs.roborev.settings.search.embeddings.api_key.file = "/nix/store/key";
  };
  C10_embedding_ambiguous = reject {
    programs.roborev.settings.search.embeddings.api_key = {
      env = "FIXTURE_KEY";
      file = "/run/key";
    };
  };
  C11_runtime_program_only = accepted runtime && runtime.programs.roborev.settings.max_workers == null;
  C12_runtime_store = reject {
    programs.roborev.runtimeConfigFile = "/nix/store/config.toml";
  };
  C13_runtime_settings_conflict = reject {
    programs.roborev.runtimeConfigFile = "/run/config.toml";
    programs.roborev.settings.max_workers = 3;
  };
  C14_typed_workers = reject {programs.roborev.settings.max_workers = "wrong";};
  C15_typed_severity = reject {programs.roborev.settings.review_min_severity = "wrong";};
  C16_repository_renderer = accepted repositories && repositories.programs.roborev.repositories.example ? configFile;
  C17_repository_scope = reject {programs.roborev.repositories.example.settings.server_addr = "unix://";};
  C18_environment = accepted (configure {
    programs.roborev.colorMode = "dark";
    programs.roborev.environment.FIXTURE_VALUE = "dollar $ percent % quote \"";
  });
  C19_reserved_environment = reject {programs.roborev.environment.PATH = "/fixture";};
  C20_inline_postgres = reject {programs.roborev.settings.sync.postgres_url = "postgres://user:fixture@host/db";};
  C21_postgres_file = accepted (configure {
    programs.roborev.settings.sync.postgres_url = "postgres://user:${"\${file:/run/pg-password}"}@host/db";
  });
  C22_inline_notification = reject {programs.roborev.settings.ci.discord_webhook_url = "https://discord.com/api/webhooks/fixture/token";};
  C23_literal_embedding_table = reject {programs.roborev.settings.search.embeddings.api_key.value = "fixture";};
  C24_inline_hook = reject {
    programs.roborev.settings.hooks = [
      {
        event = "review.failed";
        url = "https://fixture/token";
      }
    ];
  };
  C25_store_postgres = reject {programs.roborev.settings.sync.postgres_url = "postgres://user:${"\${file:/nix/store/password}"}@host/db";};
  C26_query_postgres = reject {programs.roborev.settings.sync.postgres_url = "postgres://host/db?password=fixture";};
  C27_repository_install = let
    cfg = configure {
      programs.roborev.repositories.fixture.target = "/home/roborev-fixture/project/.roborev.toml";
    };
  in
    accepted cfg && cfg.home.file.roborev-repository-fixture.target == "project/.roborev.toml" && !cfg.home.file.roborev-repository-fixture.force;
  C28_repository_target = reject {programs.roborev.repositories.fixture.target = "/outside/.roborev.toml";};
  C29_hook_profiles = builtins.all (name:
    accepted (configure {
      programs.roborev.integrations.${name} = {
        hooks.enable = true;
        mcp.enable = true;
        skills.enable = true;
      };
    })) ["claude" "codex" "copilot" "cursor" "droid" "gemini" "hermes" "qwen" "grok"];
  C30_no_opencode_hook = reject {programs.roborev.integrations.opencode.hooks.enable = true;};
  C31_shared_install = let
    cfg = configure {
      programs.roborev.integrations.gemini = {
        hooks = {
          enable = true;
          target = "/home/roborev-fixture/.gemini/settings.json";
        };
        mcp = {
          enable = true;
          target = "/home/roborev-fixture/.gemini/settings.json";
        };
      };
    };
  in
    accepted cfg && cfg.home.file ? roborev-gemini-hooks && !(cfg.home.file ? roborev-gemini-mcp);
  C32_wrong_shared_install = reject {
    programs.roborev.integrations.codex = {
      hooks = {
        enable = true;
        target = "/home/roborev-fixture/.codex/hooks.json";
      };
      mcp = {
        enable = true;
        target = "/home/roborev-fixture/.codex/hooks.json";
      };
    };
  };
  C33_http_mcp = accepted (configure {
    programs.roborev.integrations.claude.mcp = {
      enable = true;
      transport = "http";
      url = "http://127.0.0.1:7373/mcp";
    };
  });
  C34_http_credentials = reject {
    programs.roborev.integrations.claude.mcp = {
      enable = true;
      transport = "http";
      url = "http://user:fixture@host/mcp";
    };
  };
  C35_lossless_omission = let
    schema = import ../../modules/home/roborev/schema.nix {
      inherit lib;
      toml = pkgs.formats.toml {};
    };
    cfg = configure {
      programs.roborev.settings = {
        hook_timeout_seconds = 0;
        mouse_enabled = false;
        default_model = "";
        hidden_columns = [];
        projects = {};
        future_setting = "passthrough";
      };
    };
    result = schema.clean cfg.programs.roborev.settings;
  in
    result.hook_timeout_seconds == 0 && !result.mouse_enabled && result.default_model == "" && result.hidden_columns == [] && result.projects == {} && !(result ? task_column_order) && result.future_setting == "passthrough";
  C36_legacy_relative_override = accepted (configure {programs.roborev.settings.gemini_cmd = "gemini";});
  C37_normalized_values = accepted (configure {
    programs.roborev.settings = {
      review_reasoning = " HIGH ";
      review_min_severity = " LOW ";
    };
  });
  C38_runtime_self_link = reject {programs.roborev.runtimeConfigFile = "/home/roborev-fixture/.roborev/config.toml";};
  C39_empty_embedding_reference = accepted (configure {programs.roborev.settings.search.embeddings.api_key = {};});
  C40_nested_repository_scope = reject {programs.roborev.repositories.example.settings.ci.enabled = true;};
  C41_nested_policy_scope = reject {programs.roborev.repositories.example.settings.goal_review.kata_gate.default = "warn";};
  C42_repository_goal_review = accepted (configure {programs.roborev.repositories.example.settings.goal_review.watch = ["goal"];});
  C43_encoded_postgres_parameter = reject {programs.roborev.settings.sync.postgres_url = "postgresql://host/db?pass%77ord=fixture";};
  C44_keyword_postgres = reject {programs.roborev.settings.sync.postgres_url = "host=db password=fixture";};
  C45_postgres_query_reference = accepted (configure {
    programs.roborev.settings.sync.postgres_url = "postgres://user@host/db?sslmode=require&password=${"\${PG_PASSWORD}"}";
  });
  C46_http_mcp_server = reject {
    programs.roborev.integrations.claude = {
      server = "unix:///run/user/1000/roborev.sock";
      mcp = {
        enable = true;
        transport = "http";
        url = "http://127.0.0.1:7373/mcp";
      };
    };
  };
}
