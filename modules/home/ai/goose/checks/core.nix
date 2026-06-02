{inputs, ...}: {
  perSystem = {
    pkgs,
    system,
    ...
  }: let
    gooseCheckLib = inputs.self.lib.gooseCheckFixtures {
      inherit inputs pkgs system;
    };
    inherit
      (gooseCheckLib)
      mkExpectedEvalFailure
      mkExpectedEvalFailureCheck
      mkFixture
      mkGooseCheck
      mkGooseHome
      ;

    pkgsCopilot = import inputs.nixpkgs {
      inherit system;
      config.allowUnfreePredicate = pkg:
        builtins.elem (inputs.nixpkgs.lib.getName pkg) [
          "github-copilot-cli"
        ];
    };

    homes = {
      cli = mkGooseHome {
        extraModules = [
          {
            programs.bash.enable = true;
            programs.fish.enable = true;
            programs.zsh.enable = true;

            programs.goose = {
              cli.enable = true;
              acp.providers = {
                copilot = {
                  enable = true;
                  packages = [pkgsCopilot."github-copilot-cli"];
                };
              };
              defaultModel = "gpt-5.4";
              pathRoot = "/tmp/goose-cli-home";
              planner = {
                provider = "openai";
                model = "gpt-4.1";
              };
              toolshim = {
                enable = true;
                ollamaModel = "llama3.2";
              };
              permissions.user.alwaysAllow = ["developer__text_editor"];
              prompts.templates."plan.md".text = "Custom plan prompt";
              recipes.paths = [
                "/tmp/goose-test/recipes"
                "/srv/shared/goose-recipes"
              ];
              recipes.githubRepo = "block/goose-recipes";
              predefinedModels = [
                {
                  name = "custom-model";
                  provider = "openai";
                  contextLimit = 123456;
                  requestParams.reasoning = true;
                }
              ];
              providers = {
                openai = {
                  default = true;
                  settings.OPENAI_BASE_PATH = "/v1/chat/completions";
                  secretFiles.OPENAI_API_KEY = pkgs.writeText "openai-key" "openai-secret\n";
                };

                ollama.settings.OLLAMA_HOST = "http://127.0.0.1:11434";

                chatgpt_codex = {};
              };
              customProviders."local-swap" = {
                name = "local-swap";
                engine = "openai";
                display_name = "Local Swap";
                base_url = "http://127.0.0.1:8013/v1";
                models = [
                  {
                    name = "qwen3-coder-next";
                    context_limit = 65536;
                  }
                ];
                supports_streaming = true;
                requires_auth = false;
              };
              allowlist = {
                url = "https://example.com/allowlist.json";
                warningMode = true;
              };
              searchPaths = [
                "/opt/tools"
                "~/custom/tools"
              ];
              developer.shell = "/bin/zsh";
              moim.text = "Injected context";
              promptEditor = {
                command = "code --wait";
                always = true;
              };
              cli = {
                theme = "ansi";
                lightTheme = "GitHub";
                darkTheme = "zenburn";
                showCost = true;
                minPriority = 0.2;
                newlineKey = "n";
              };
              session = {
                maxTurns = 42;
                autoCompactThreshold = 0.7;
                maxActiveAgents = 3;
                disableSessionNaming = true;
              };
              subagents.maxTurns = 17;
              telemetry.enable = false;
              security.promptInjection = {
                enable = true;
                threshold = 0.9;
                classifier = {
                  enable = true;
                  endpoint = "https://example.com/classify";
                };
              };
              terminalIntegration = {
                enable = true;
                sessionName = "goose-smoke";
                commandNotFound = true;
              };
              environmentSecretFiles.GOOSE_EDITOR_API_KEY = pkgs.writeText "editor-key" "editor-secret\n";
            };
          }
        ];
      };

      desktop = mkGooseHome {
        extraModules = [
          {
            programs.goose = {
              cli.enable = true;
              desktop.enable = true;
              providers.openai.default = true;
              defaultModel = "gpt-5.4";
              pathRoot = "/tmp/goose-desktop-home";
              allowlist = {
                url = "https://example.com/allowlist.json";
                warningMode = true;
              };
            };
          }
        ];
      };

      extension = mkGooseHome {
        extraModules = [
          {
            programs.goose = {
              cli.enable = true;
              desktop.enable = true;
              providers.openai.default = true;
              defaultModel = "gpt-5.4";
              pathRoot = "/tmp/goose-extension-home";
              searchPaths = ["/opt/tools"];
              extensions = {
                developer = {
                  enable = true;
                  type = "builtin";
                  bundled = true;
                  displayName = "Developer";
                  timeout = 300;
                  availableTools = ["developer__text_editor"];
                };

                memory = {
                  enable = true;
                  type = "stdio";
                  command = "jq";
                  args = ["--version"];
                  packages = [pkgs.jq];
                  environment.MEMORY_STORE = "/var/lib/goose-memory";
                  secretFiles.AUTH_TOKEN = pkgs.writeText "goose-memory-token" "memory-secret\n";
                  timeout = 123;
                  availableTools = ["memory_read_graph"];
                };

                "Flight Search" = {
                  enable = true;
                  type = "streamableHttp";
                  uri = "https://mcp.example.com/stream?token=$HTTP_TOKEN";
                  headers.Authorization = "Bearer $HTTP_TOKEN";
                  secretFiles.HTTP_TOKEN = pkgs.writeText "goose-http-token" "http-secret\n";
                  timeout = 45;
                  availableTools = ["search_flights"];
                };

                workspace-platform = {
                  enable = true;
                  type = "platform";
                  displayName = "Workspace Platform";
                  availableTools = ["workspace_summary"];
                };
              };
            };
          }
        ];
      };

      cursor = mkGooseHome {
        extraModules = [
          {
            programs.goose = {
              cli.enable = true;
              pathRoot = "/tmp/goose-cursor-home";
              agent.providers.cursor = {
                enable = true;
                default = true;
              };
              defaultModel = "auto";
            };
          }
        ];
      };
    };

    fixtures = builtins.mapAttrs (_: mkFixture) homes;

    expectedFailures = {
      multiDefault = mkExpectedEvalFailure [
        {
          programs.goose = {
            providers.openai.default = true;
            providers.ollama.default = true;
            defaultModel = "qwen";
          };
        }
      ];

      invalidAcpKey = mkExpectedEvalFailure [
        {
          programs.goose = {
            pathRoot = "/tmp/goose-invalid-acp-key";
            acp.providers."claude-acp".enable = true;
            settings.GOOSE_MODE = "auto";
          };
        }
      ];

      typedRawExtensions = mkExpectedEvalFailure [
        {
          programs.goose = {
            pathRoot = "/tmp/goose-typed-raw-extensions";
            extensions.developer = {
              enable = true;
              type = "builtin";
            };
            settings.extensions.raw = {
              enabled = true;
              name = "raw";
              type = "builtin";
            };
          };
        }
      ];

      providerExtensionSecretReuse = mkExpectedEvalFailure [
        {
          programs.goose = {
            pathRoot = "/tmp/goose-provider-extension-secret-reuse";
            providers.openai.secretFiles.OPENAI_API_KEY = pkgs.writeText "goose-openai-key" "openai-secret\n";
            extensions.memory = {
              enable = true;
              type = "stdio";
              command = "jq";
              secretFiles.OPENAI_API_KEY = pkgs.writeText "goose-extension-openai-key" "extension-secret\n";
            };
          };
        }
      ];

      managedSearchPathSettings = mkExpectedEvalFailure [
        {
          programs.goose = {
            pathRoot = "/tmp/goose-search-settings-conflict";
            extensions.memory = {
              enable = true;
              type = "stdio";
              command = "jq";
              packages = [pkgs.jq];
            };
            settings.GOOSE_SEARCH_PATHS = ["/tmp/raw-search-path"];
          };
        }
      ];

      managedSearchPathEnvironment = mkExpectedEvalFailure [
        {
          programs.goose = {
            pathRoot = "/tmp/goose-search-environment-conflict";
            extensions.memory = {
              enable = true;
              type = "stdio";
              command = "jq";
              packages = [pkgs.jq];
            };
            environment.GOOSE_SEARCH_PATHS = "/tmp/raw-search-path";
          };
        }
      ];
    };

    cliConfigSmoke = mkGooseCheck {
      name = "goose-cli-config-smoke";
      fixture = fixtures.cli;
      body = ''
        config_src="$(jq -r '.editableFiles["config.yaml"]' "$spec")"
        test -f "$config_src"

        test "$(yq -r '.GOOSE_PROVIDER' "$config_src")" = "openai"
        test "$(yq -r '.GOOSE_MODEL' "$config_src")" = "gpt-5.4"
        test "$(yq -r '.GOOSE_PLANNER_PROVIDER' "$config_src")" = "openai"
        test "$(yq -r '.GOOSE_PLANNER_MODEL' "$config_src")" = "gpt-4.1"
        test "$(yq -r '.GOOSE_TOOLSHIM' "$config_src")" = "true"
        test "$(yq -r '.GOOSE_TOOLSHIM_OLLAMA_MODEL' "$config_src")" = "llama3.2"
        test "$(yq -r '.GOOSE_RECIPE_GITHUB_REPO' "$config_src")" = "block/goose-recipes"
        test "$(yq -r '.GOOSE_PROMPT_EDITOR' "$config_src")" = "code --wait"
        test "$(yq -r '.GOOSE_PROMPT_EDITOR_ALWAYS' "$config_src")" = "true"
        test "$(yq -r '.GOOSE_CLI_THEME' "$config_src")" = "ansi"
        test "$(yq -r '.GOOSE_CLI_LIGHT_THEME' "$config_src")" = "GitHub"
        test "$(yq -r '.GOOSE_CLI_DARK_THEME' "$config_src")" = "zenburn"
        test "$(yq -r '.GOOSE_CLI_SHOW_COST' "$config_src")" = "true"
        test "$(yq -r '.GOOSE_CLI_MIN_PRIORITY' "$config_src")" = "0.2"
        test "$(yq -r '.GOOSE_CLI_NEWLINE_KEY' "$config_src")" = "n"
        test "$(yq -r '.GOOSE_MAX_TURNS' "$config_src")" = "42"
        test "$(yq -r '.GOOSE_AUTO_COMPACT_THRESHOLD' "$config_src")" = "0.7"
        test "$(yq -r '.GOOSE_MAX_ACTIVE_AGENTS' "$config_src")" = "3"
        test "$(yq -r '.GOOSE_DISABLE_SESSION_NAMING' "$config_src")" = "true"
        test "$(yq -r '.GOOSE_SUBAGENT_MAX_TURNS' "$config_src")" = "17"
        test "$(yq -r '.GOOSE_TELEMETRY_ENABLED' "$config_src")" = "false"
        test "$(yq -r '.SECURITY_PROMPT_ENABLED' "$config_src")" = "true"
        test "$(yq -r '.SECURITY_PROMPT_THRESHOLD' "$config_src")" = "0.9"
        test "$(yq -r '.SECURITY_PROMPT_CLASSIFIER_ENABLED' "$config_src")" = "true"
        test "$(yq -r '.SECURITY_PROMPT_CLASSIFIER_ENDPOINT' "$config_src")" = "https://example.com/classify"
        test "$(yq -r '.GOOSE_DISABLE_KEYRING' "$config_src")" = "true"
        test "$(yq -r '.GOOSE_SEARCH_PATHS[0]' "$config_src")" = "/opt/tools"
        test "$(yq -r '.GOOSE_SEARCH_PATHS[1]' "$config_src")" = "~/custom/tools"
        test "$(yq -r '.OPENAI_BASE_PATH' "$config_src")" = "/v1/chat/completions"
        test "$(yq -r '.OLLAMA_HOST' "$config_src")" = "http://127.0.0.1:11434"
        test "$(yq -r '."chatgpt_codex_configured" // ""' "$config_src")" = ""

        "$stateScript" "$spec" "$TMPDIR/goose-state.json"

        test -f "$config_dir/config.yaml"
        ! test -L "$config_dir/config.yaml"
        test -f "$config_dir/permission.yaml"
        ! test -L "$config_dir/permission.yaml"
        test "$(yq -r '.user.always_allow[0]' "$config_dir/permission.yaml")" = "developer__text_editor"
        test -f "$config_dir/secrets.yaml"
        test "$(yq -r '.OPENAI_API_KEY' "$config_dir/secrets.yaml")" = "openai-secret"
        test "$(stat -c '%a' "$config_dir/secrets.yaml")" = "600"
        test -f "$config_dir/custom_providers/local-swap.json"
        ! test -L "$config_dir/custom_providers/local-swap.json"
        test "$(yq -r '.base_url' "$config_dir/custom_providers/local-swap.json")" = "http://127.0.0.1:8013/v1"
        test -f "$config_dir/.home-manager-environment.sh"
        grep -Fqx "export GOOSE_EDITOR_API_KEY=editor-secret" "$config_dir/.home-manager-environment.sh"
      '';
    };

    extensionConfigSmoke = mkGooseCheck {
      name = "goose-extension-config-smoke";
      fixture = fixtures.extension;
      body = ''
        config_src="$(jq -r '.editableFiles["config.yaml"]' "$spec")"
        test -f "$config_src"

        test "$(yq -r '.GOOSE_PROVIDER' "$config_src")" = "openai"
        test "$(yq -r '.GOOSE_MODEL' "$config_src")" = "gpt-5.4"
        test "$(yq -r '.GOOSE_DISABLE_KEYRING' "$config_src")" = "true"
        test "$(yq -r '.GOOSE_SEARCH_PATHS[0]' "$config_src")" = "${pkgs.jq}/bin"
        test "$(yq -r '.GOOSE_SEARCH_PATHS[1]' "$config_src")" = "/opt/tools"

        test "$(yq -r '.extensions.developer.enabled' "$config_src")" = "true"
        test "$(yq -r '.extensions.developer.type' "$config_src")" = "builtin"
        test "$(yq -r '.extensions.developer.display_name' "$config_src")" = "Developer"
        test "$(yq -r '.extensions.developer.timeout' "$config_src")" = "300"
        test "$(yq -r '.extensions.developer.available_tools[0]' "$config_src")" = "developer__text_editor"

        test "$(yq -r '.extensions.memory.type' "$config_src")" = "stdio"
        test "$(yq -r '.extensions.memory.cmd' "$config_src")" = "jq"
        test "$(yq -r '.extensions.memory.args[0]' "$config_src")" = "--version"
        test "$(yq -r '.extensions.memory.timeout' "$config_src")" = "123"
        test "$(yq -r '.extensions.memory.envs.MEMORY_STORE' "$config_src")" = "/var/lib/goose-memory"
        test "$(yq -r '.extensions.memory.env_keys[0]' "$config_src")" = "AUTH_TOKEN"
        test "$(yq -r '.extensions.memory.available_tools[0]' "$config_src")" = "memory_read_graph"

        test "$(yq -r '.extensions.flightsearch.type' "$config_src")" = "streamable_http"
        test "$(yq -r '.extensions.flightsearch.uri' "$config_src")" = 'https://mcp.example.com/stream?token=$HTTP_TOKEN'
        test "$(yq -r '.extensions.flightsearch.headers.Authorization' "$config_src")" = 'Bearer $HTTP_TOKEN'
        test "$(yq -r '.extensions.flightsearch.env_keys[0]' "$config_src")" = "HTTP_TOKEN"
        test "$(yq -r '.extensions.flightsearch.timeout' "$config_src")" = "45"
        test "$(yq -r '.extensions.flightsearch.available_tools[0]' "$config_src")" = "search_flights"

        test "$(yq -r '.extensions."workspace-platform".type' "$config_src")" = "platform"
        test "$(yq -r '.extensions."workspace-platform".display_name' "$config_src")" = "Workspace Platform"
        test "$(yq -r '.extensions."workspace-platform".available_tools[0]' "$config_src")" = "workspace_summary"

        ! grep -F "memory-secret" "$config_src"
        ! grep -F "http-secret" "$config_src"

        "$stateScript" "$spec" "$TMPDIR/goose-extension-state.json"

        test -f "$config_dir/config.yaml"
        ! test -L "$config_dir/config.yaml"
        test -f "$config_dir/secrets.yaml"
        test "$(yq -r '.AUTH_TOKEN' "$config_dir/secrets.yaml")" = "memory-secret"
        test "$(yq -r '.HTTP_TOKEN' "$config_dir/secrets.yaml")" = "http-secret"
        test "$(stat -c '%a' "$config_dir/secrets.yaml")" = "600"
      '';
    };

    copilotPackageSmoke =
      pkgs.runCommand "goose-copilot-package-smoke" {
        expectedPath = builtins.unsafeDiscardStringContext pkgsCopilot."github-copilot-cli".outPath;
        inherit (fixtures.cli) packagePaths;
      } ''
        printf '%s' "$packagePaths" | grep -F "$expectedPath" >/dev/null
        touch $out
      '';

    extensionPackageSmoke =
      pkgs.runCommand "goose-extension-package-smoke" {
        expectedPath = builtins.unsafeDiscardStringContext pkgs.jq.outPath;
        inherit (fixtures.extension) packagePaths;
      } ''
        printf '%s' "$packagePaths" | grep -F "$expectedPath" >/dev/null
        touch $out
      '';

    editableFilesReapplySmoke = mkGooseCheck {
      name = "goose-editable-files-reapply-smoke";
      fixture = fixtures.cli;
      body = ''
        "$stateScript" "$spec" "$TMPDIR/goose-reapply-state.json"

        printf 'GOOSE_PROVIDER: atlas-swap\nGOOSE_MODEL: gemma4-31b\n' >"$config_dir/config.yaml"
        printf 'user:\n  always_allow:\n    - mutated-tool\n' >"$config_dir/permission.yaml"
        printf '{\n  "base_url": "http://mutated.example/v1"\n}\n' >"$config_dir/custom_providers/local-swap.json"

        test "$(yq -r '.GOOSE_PROVIDER' "$config_dir/config.yaml")" = "atlas-swap"
        test "$(yq -r '.user.always_allow[0]' "$config_dir/permission.yaml")" = "mutated-tool"
        test "$(jq -r '.base_url' "$config_dir/custom_providers/local-swap.json")" = "http://mutated.example/v1"

        "$stateScript" "$spec" "$TMPDIR/goose-reapply-state.json"

        test -f "$config_dir/config.yaml"
        ! test -L "$config_dir/config.yaml"
        test "$(yq -r '.GOOSE_PROVIDER' "$config_dir/config.yaml")" = "openai"
        test "$(yq -r '.GOOSE_MODEL' "$config_dir/config.yaml")" = "gpt-5.4"

        test -f "$config_dir/permission.yaml"
        ! test -L "$config_dir/permission.yaml"
        test "$(yq -r '.user.always_allow[0]' "$config_dir/permission.yaml")" = "developer__text_editor"

        test -f "$config_dir/custom_providers/local-swap.json"
        ! test -L "$config_dir/custom_providers/local-swap.json"
        test "$(jq -r '.base_url' "$config_dir/custom_providers/local-swap.json")" = "http://127.0.0.1:8013/v1"
      '';
    };

    multiDefaultAssertionSmoke = mkExpectedEvalFailureCheck {
      name = "goose-multi-default-assertion";
      result = expectedFailures.multiDefault;
    };

    invalidAcpKeyAssertionSmoke = mkExpectedEvalFailureCheck {
      name = "goose-invalid-acp-key-assertion";
      result = expectedFailures.invalidAcpKey;
    };

    typedRawExtensionsAssertionSmoke = mkExpectedEvalFailureCheck {
      name = "goose-typed-raw-extensions-assertion";
      result = expectedFailures.typedRawExtensions;
    };

    providerExtensionSecretReuseAssertionSmoke = mkExpectedEvalFailureCheck {
      name = "goose-provider-extension-secret-reuse-assertion";
      result = expectedFailures.providerExtensionSecretReuse;
    };

    managedSearchPathSettingsAssertionSmoke = mkExpectedEvalFailureCheck {
      name = "goose-managed-search-path-settings-assertion";
      result = expectedFailures.managedSearchPathSettings;
    };

    managedSearchPathEnvironmentAssertionSmoke = mkExpectedEvalFailureCheck {
      name = "goose-managed-search-path-environment-assertion";
      result = expectedFailures.managedSearchPathEnvironment;
    };
  in {
    checks = {
      goose-cli-package = fixtures.cli.home.config.programs.goose.cli.package;
      goose-desktop-package = fixtures.desktop.home.config.programs.goose.desktop.package;
      goose-cli-home = fixtures.cli.home.activationPackage;
      goose-cli-config = cliConfigSmoke;
      goose-copilot-package = copilotPackageSmoke;
      goose-cursor-home = fixtures.cursor.home.activationPackage;
      goose-desktop-home = fixtures.desktop.home.activationPackage;
      goose-editable-files-reapply = editableFilesReapplySmoke;
      goose-extension-home = fixtures.extension.home.activationPackage;
      goose-extension-config = extensionConfigSmoke;
      goose-extension-package = extensionPackageSmoke;
      goose-multi-default-assertion = multiDefaultAssertionSmoke;
      goose-invalid-acp-key-assertion = invalidAcpKeyAssertionSmoke;
      goose-typed-raw-extensions-assertion = typedRawExtensionsAssertionSmoke;
      goose-provider-extension-secret-reuse-assertion = providerExtensionSecretReuseAssertionSmoke;
      goose-managed-search-path-settings-assertion = managedSearchPathSettingsAssertionSmoke;
      goose-managed-search-path-environment-assertion = managedSearchPathEnvironmentAssertionSmoke;
    };
  };
}
