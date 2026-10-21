# Roborev v0.72.0 / 1d6571d0993d7d56ba0d7da312d27191b881c913.
# Field names are checked against the pinned Go structs, including Kit's Embedder.
{
  lib,
  toml,
}: let
  inherit (lib) types;
  names = value: lib.splitString " " value;
  fields = type: value: lib.genAttrs (names value) (_: type);
  optional = path: type:
    lib.mkOption {
      type = types.nullOr type;
      default = null;
      description = "Upstream `${path}` setting. Omit to inherit Roborev's defaults and configuration layers. See https://www.roborev.io/docs/configuration/.";
    };
  record = name:
    types.submodule {
      freeformType = toml.type;
      options = lib.mapAttrs (key: optional "${name}.${key}") records.${name};
    }
    // {roborevRecord = name;};
  strings = fields types.str;
  integers = fields types.int;
  booleans = fields types.bool;
  lists = fields (types.listOf types.str);
  recordMap = name: types.attrsOf (record name);
  normalizedEnum = values: types.addCheck types.str (value: builtins.elem (lib.toLower (lib.trim value)) values);
  reasoning = normalizedEnum ["" "fast" "standard" "thorough" "maximum" "low" "medium" "high" "xhigh" "max"];
  severity = normalizedEnum ["" "critical" "high" "medium" "low"];
  workflows = ["review" "refine" "fix" "security" "design"];
  levels = ["" "fast" "low" "standard" "medium" "thorough" "high" "xhigh" "maximum" "max"];
  routes = builtins.listToAttrs (lib.concatMap (
      workflow:
        (lib.concatMap (kind:
          map (level: {
            name = "${workflow}_${kind}${lib.optionalString (level != "") "_${level}"}";
            value = types.str;
          })
          levels) ["agent" "model"])
        ++ map (kind: {
          name = "${workflow}_backup_${kind}";
          value = types.str;
        }) ["agent" "model"]
    )
    workflows);
  policy =
    fields reasoning "review_reasoning refine_reasoning fix_reasoning classify_reasoning"
    // fields severity "review_min_severity refine_min_severity fix_min_severity";
  classifier = strings "classify_agent classify_model classify_backup_agent classify_backup_model";
  shared =
    routes
    // policy
    // classifier
    // {
      experiments = recordMap "ExperimentDefinition";
      analyze = recordMap "AnalyzeConfig";
      hooks = types.listOf (record "HookConfig");
      review = record "ReviewConfig";
      kata_context = record "KataContextConfig";
      acp = recordMap "ACPAgentConfig";
    };
  records = {
    Config =
      shared
      // strings "auth_key server_addr review_guidelines fix_guidelines default_agent default_model default_backup_agent default_backup_model agent_quota_cooldown fix_commit_author codex_cmd claude_code_cmd gemini_cmd cursor_cmd pi_cmd opencode_cmd grok_cmd anthropic_api_key"
      // integers "max_workers review_context_count reuse_review_session_lookback job_timeout_minutes hook_timeout_seconds default_max_prompt_size tab_width column_config_version prompt_retention_days job_log_retention_days"
      // booleans "isolate_reviews disable_codex_sandbox auto_close_passing_reviews hide_closed_by_default hide_addressed_by_default show_classify_jobs mouse_enabled column_borders allow_unsafe_agents reuse_review_session"
      // lists "fix_commit_co_authored_by exclude_patterns hidden_columns column_order task_column_order"
      // {
        projects = recordMap "ProjectConfig";
        sync = record "SyncConfig";
        ci = record "CIConfig";
        cost = record "CostConfig";
        web = record "WebConfig";
        tui = record "TUIConfig";
        mcp = record "MCPConfig";
        search = record "SearchConfig";
        budget = record "BudgetConfig";
        agent = record "AgentConfig";
        auto_design_review = record "AutoDesignReviewConfig";
        agent_hook = record "AgentHookConfig";
        droid_hook = record "DroidHookConfig";
        advanced = record "AdvancedConfig";
        remote = record "RemoteConfig";
        remote_client = record "RemoteClientConfig";
        daemon_tls = record "DaemonTLSConfig";
        goal_review = record "GoalReviewPolicyConfig";
      };
    RepoConfig =
      shared
      // strings "agent model backup_agent backup_model review_guidelines display_name fix_commit_author snapshot_dir"
      // integers "review_context_count job_timeout_minutes hook_timeout_seconds post_commit_batch_size reuse_review_session_lookback max_prompt_size"
      // booleans "review_md_fallback isolate_reviews reuse_review_session auto_close_passing_reviews show_classify_jobs review_guidelines_supersede_global"
      // lists "excluded_branches excluded_branch_patterns excluded_commit_patterns fix_commit_co_authored_by exclude_patterns"
      // {
        post_commit_review = types.enum ["" "commit" "branch"];
        ci = record "RepoCIConfig";
        auto_design_review = record "AutoDesignReviewRepoConfig";
        goal_review = record "GoalReviewConfig";
      };
    ProjectConfig =
      strings "review_model display_name synthesis_model"
      // booleans "override_panel_models override_panel_reasoning"
      // fields reasoning "review_reasoning synthesis_reasoning";
    ExperimentDefinition =
      booleans "enabled"
      // {
        ratio = types.addCheck (types.either types.int types.float) (value: value >= 0 && value <= 1);
        workflows = types.listOf (types.enum ["review" "ci"]);
        config = toml.type;
      };
    AnalyzeConfig = strings "agent model" // {inherit reasoning;};
    HookConfig = strings "event command type url project" // lists "branches labels" // integers "priority";
    SyncConfig =
      booleans "enabled"
      // strings "postgres_url interval machine_name connect_timeout"
      // {
        repo_names = types.attrsOf types.str;
      };
    GitHubAppConfig =
      strings "github_api_url github_app_private_key"
      // integers "github_app_id github_app_installation_id"
      // {github_app_installations = types.attrsOf types.int;};
    CIConfig =
      records.GitHubAppConfig
      // booleans "enabled upsert_comments include_costs"
      // strings "poll_interval throttle_interval model synthesis_agent synthesis_backup_agent synthesis_model panel discord_webhook_url batch_timeout"
      // lists "repos exclude_repos skip_labels review_types agents throttle_bypass_users"
      // integers "max_repos"
      // {
        reviews = types.attrsOf (types.listOf types.str);
        quiet_hours = record "QuietHoursConfig";
        min_severity = severity;
      };
    QuietHoursConfig = strings "start end timezone throttle_interval" // lists "bypass_users";
    RepoCIConfig =
      lists "agents review_types"
      // strings "panel"
      // booleans "upsert_comments include_costs"
      // {
        reviews = types.attrsOf (types.listOf types.str);
        inherit reasoning;
        min_severity = severity;
      };
    CostConfig = strings "endpoint timeout";
    WebConfig = booleans "enabled" // strings "listen public_origin base_path auth_mode auth_token auth_token_file";
    TUIConfig = booleans "filter_repo filter_branch";
    MCPConfig = booleans "enabled";
    SearchConfig = {embeddings = record "Embedder";};
    Embedder =
      strings "base_url model fingerprint_salt"
      // integers "dims batch_size model_context_tokens max_batch_tokens timeout_seconds"
      // booleans "trust_private_network"
      // {
        input_type_mode = types.enum ["" "none" "retrieval"];
        api_key = types.either types.str (record "Ref");
      };
    Ref = strings "value env file";
    BudgetConfig =
      booleans "enabled"
      // integers "daily_limit_cents reserve_floor_cents"
      // {
        agent_costs = types.attrsOf types.int;
      };
    AgentConfig = {
      codex = record "CodexConfig";
      grok = record "GrokConfig";
      pi = record "PiConfig";
    };
    CodexConfig = booleans "disable_review_skills ignore_review_user_config" // {config = toml.type;};
    GrokConfig = strings "sandbox";
    PiConfig = strings "jsonschemaextension" // lists "launch_args";
    AutoDesignReviewConfig =
      booleans "enabled hook_enabled"
      // integers "min_diff_lines large_diff_lines large_file_count classifier_timeout_seconds classifier_max_prompt_size"
      // lists "trigger_paths skip_paths trigger_message_patterns skip_message_patterns";
    AutoDesignReviewRepoConfig = records.AutoDesignReviewConfig;
    ReviewConfig =
      strings "default_panel hook_review_panel"
      // {
        types = recordMap "ReviewTypeSpec";
        subagents = recordMap "SubagentSpec";
        panels = recordMap "PanelSpec";
      };
    ReviewTypeSpec =
      strings "template agent model"
      // {
        includes = types.attrsOf types.str;
        inherit reasoning;
      };
    SubagentSpec =
      strings "agent model provider review_type instructions timeout"
      // booleans "allow_failure non_voting" // {inherit reasoning;};
    PanelSpec =
      lists "members"
      // strings "synthesis_agent synthesis_model synthesis_backup_agent synthesis_backup_model"
      // {synthesis_reasoning = reasoning;};
    AgentHookConfig = integers "turn_threshold commit_threshold failed_review_threshold" // strings "instruction";
    DroidHookConfig = records.AgentHookConfig;
    KataContextConfig = {
      mode = types.enum ["" "off" "current" "open"];
      max_chars = types.int;
    };
    AdvancedConfig = booleans "tasks_enabled";
    RemoteConfig =
      booleans "enabled trusted_proxy"
      // strings "listen external_url cert_file tls_key_file read_header_timeout read_timeout request_timeout"
      // integers "max_header_bytes max_concurrent"
      // {keys = types.listOf (record "RemoteSigningKey");};
    RemoteSigningKey =
      strings "id secret_file secret_env"
      // lists "grants"
      // booleans "all_repos"
      // {repo_ids = types.listOf types.int;};
    RemoteClientConfig = strings "external_url key_id secret_file secret_env ca_file";
    DaemonTLSConfig = strings "ca_file cert_file key_file client_cert_file client_key_file";
    GoalReviewPolicyConfig = {kata_gate = record "GoalReviewGateConfig";};
    GoalReviewGateConfig.default = types.enum ["block" "warn" "off"];
    GoalReviewConfig =
      booleans "enabled"
      // strings "spec_file plan_file"
      // {watch = types.listOf (types.enum ["goal" "kata_graph"]);};
    ACPAgentConfig =
      strings "command read_only_mode auto_approve_mode mode model"
      // lists "args" // integers "timeout" // booleans "disable_mode_negotiation";
  };
  # Null denotes absence, not a TOML value. Preserve false/0/empty collections.
  clean = value:
    if builtins.isAttrs value
    then lib.mapAttrs (_: clean) (lib.filterAttrs (_: v: v != null) value)
    else if builtins.isList value
    then map clean value
    else value;
in {
  globalType = record "Config";
  repositoryType = record "RepoConfig";
  # Paths valid only globally, descending into records that differ by scope.
  globalOnlyPaths = let
    walk = prefix: global: repository:
      lib.concatLists (lib.mapAttrsToList (
          key: type: let
            other = repository.${key};
          in
            if !(repository ? ${key})
            then [(prefix ++ [key])]
            else if type ? roborevRecord && other ? roborevRecord && type.roborevRecord != other.roborevRecord
            then walk (prefix ++ [key]) records.${type.roborevRecord} records.${other.roborevRecord}
            else []
        )
        global);
  in
    walk [] records.Config records.RepoConfig;
  inventory = lib.mapAttrs (_: builtins.attrNames) records;
  inherit clean;
}
