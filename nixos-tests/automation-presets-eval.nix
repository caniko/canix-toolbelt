{pkgs}: let
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  eval = import "${pkgs.path}/nixos/lib/eval-config.nix" {
    system = pkgs.stdenv.hostPlatform.system;
    modules = [
      ../modules/nixos/services/windmill.nix
      ../modules/nixos/services/n8n.nix
      {
        # n8n is unfree (Sustainable Use License); consumers opt in.
        nixpkgs.config.allowUnfreePredicate = p: pkgs.lib.getName p == "n8n";
        canix-toolbelt.services.windmill = {
          enable = true;
          baseUrl = "https://windmill.example.com";
          port = 8011;
          workers = 3;
        };
        canix-toolbelt.services.n8n = {
          enable = true;
          baseUrl = "https://n8n.example.com";
          port = 5690;
          encryptionKeyFile = "/run/agenix/n8n-key";
          runnersAuthTokenFile = "/run/agenix/n8n-runners";
          environment.N8N_LISTEN_ADDRESS = "0.0.0.0"; # must not widen the listener
        };
      }
    ];
  };
  units = eval.config.systemd.services;
  n8n = units.n8n.environment;
in
  mkEvalCheck {
    name = "automation-presets-eval";
    assertions = [
      {
        name = "windmill-loopback";
        assertion = units.windmill-server.environment.SERVER_BIND_ADDR == "127.0.0.1" && units.windmill-server.environment.PORT == "8011";
        message = "Windmill must listen on loopback at the configured port";
      }
      {
        name = "windmill-workers-reach-server";
        assertion = builtins.all (u: units.${u}.environment.BASE_INTERNAL_URL == "http://127.0.0.1:8011") ["windmill-worker" "windmill-worker-native"];
        message = "Workers must target the configured server port";
      }
      {
        name = "windmill-native-mode";
        assertion = units.windmill-worker-native.environment.NATIVE_MODE == "true" && units.windmill-worker.environment.NUM_WORKERS == "3";
        message = "Native group needs NATIVE_MODE and the default group NUM_WORKERS";
      }
      {
        name = "n8n-postgres-socket";
        assertion = n8n.DB_TYPE == "postgresdb" && n8n.DB_POSTGRESDB_HOST == "/run/postgresql" && builtins.elem "n8n" eval.config.services.postgresql.ensureDatabases;
        message = "n8n must use the local PostgreSQL socket";
      }
      {
        name = "n8n-secrets-are-credentials";
        assertion = n8n.N8N_ENCRYPTION_KEY_FILE == "%d/n8n_encryption_key_file" && builtins.elem "n8n_encryption_key_file:/run/agenix/n8n-key" units.n8n.serviceConfig.LoadCredential;
        message = "n8n secrets must arrive via LoadCredential, never inline";
      }
      {
        name = "n8n-public-origin";
        assertion = n8n.N8N_LISTEN_ADDRESS == "127.0.0.1" && n8n.WEBHOOK_URL == "https://n8n.example.com/" && n8n.N8N_HOST == "n8n.example.com" && units ? n8n-task-runner;
        message = "n8n must be loopback-only with public webhook origin and external runners";
      }
    ];
  }
