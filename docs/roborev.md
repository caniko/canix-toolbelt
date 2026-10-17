# Roborev ownership and consumption

Toolbelt owns the reusable Roborev package recipe, Home Manager program/service
modules, daemon transport, durable dispatch/publication contracts and Linux
worker. The standalone `opencode-repo-review-pr` client owns OpenCode commands and
presentation. Consumers supply credentials, trusted policy, executable paths,
resource values, fleet placement and qualified dependency pins.

## Nix interfaces

- `homeModules.roborev` and `homeManagerModules.roborev` export the generic module.
  Set `programs.roborev.package` and approved `agentCommands` explicitly. Both
  the program and service default to disabled. The module has no `osConfig`
  dependency and does not enroll repositories or migrate state. Native harness
  artifacts and repository files are opt-in, with explicit installation targets.
- `lib.mkRoborevPackage { pkgs; harborGo; harborJs; buildPkgs ? pkgs.buildPackages; }`
  builds stock Roborev v0.71.0 with the selected Go/Bun helpers. Bun 1.3.14 installs
  the frozen dependencies; native Node 24.2+ runs generation, typechecking, bundling
  and asset verification without PRoot tracing. Consumer package sets remain
  explicit, including cross-build native tools.
- `lib.mkRoborevFlakeModule { nixpkgs; harborGo; harborJs; harborMeta; homeManager; }`
  composes the package, frontend/dependency artifacts, development shell and
  module/unit regression checks. `flakeModules.roborev` selects those arguments
  from conventional consumer inputs. Toolbelt qualifies its own composition too.
- `lib.roborevTests` exposes the reusable evaluation/activation/unit/switch fixtures.
  Runtime scripts under `tests/roborev/` use disposable homes and isolated
  namespaces. Set `ROBOREV_TEST_TMPDIR` when a short fixture root is required.

## Home Manager configuration coverage

The schema follows Roborev v0.71.0, revision
`2087d5abc7a65f2d6de555ae49fc55795aefb78f`, including its locked Kit embedding
and secret-reference structs. `settings` declares every global field and nested
table; `repositories.<name>.settings` declares the repository fields. This
includes workflow/model/reasoning routing, fallback agents, custom review types,
panels, experiments, project overrides, CI review matrices and GitHub App
installations, budgets, sync, browser/MCP listeners, TUI settings, automatic design
review, hook policies and named ACP agents. Deprecated upstream fields remain
available for compatibility. Unknown non-secret TOML keys still pass through.

All upstream options are nullable: `null` means omitted. Explicit `false`, `0`,
empty strings, lists and tables remain present in TOML. The existing module
defaults remain `mkDefault` values and can be overridden or omitted with `null`.
Reading `programs.roborev.settings` now includes nullable option defaults; only
the cleaned settings are serialized. Upstream `config validate --global` remains
the runtime semantic validator, including constraints between settings.

```nix
programs.roborev = {
  enable = true;
  package = roborevPackage;
  defaultAgent = "gemini";
  agentCommands.gemini = "${geminiPackage}/bin/gemini";
  colorMode = "dark";
  settings = {
    default_model = "selected-model";
    review_reasoning = "high";
    agent_hook.turn_threshold = 0;
    search.embeddings = {
      base_url = "http://127.0.0.1:8080/v1";
      model = "selected-embedding-model";
      dims = 768;
      api_key.file = "/run/user/1000/embedding-key";
    };
  };
};
```

`agentCommands` accepts Codex, Claude Code (`claude-code`), Gemini, Copilot,
Cursor, OpenCode, Droid, Kilo, Kiro, Pi and Grok. The seven upstream `*_cmd`
overrides are used directly. For agents without an upstream override, the wrapper
provides the native executable name on its deterministic PATH. Raw command
settings remain supported; a raw override and `agentCommands` for the same agent
must agree. Selecting a default through a raw override requires an absolute path.
Use `extraPackages` for additional tools and service dependencies.

For ACP, select `defaultAgent = "acp.reviewer"` and configure
`settings.acp.reviewer = { command = "/absolute/acp-agent"; args = ["--stdio"];
read_only_mode = "plan"; model = "selected-model"; };`. Mode negotiation,
auto-approval, timeout and additional upstream ACP fields are available in the
same table. Mode values are specific to the selected ACP server.

### Credentials and complete runtime configuration

Generated TOML accepts upstream credential references:

- `anthropic_api_key = "\${ENV_NAME}"`;
- `ci.github_app_private_key` as an absolute runtime filename or `"\${ENV_NAME}"`;
- `web.auth_token_file` as an absolute runtime filename;
- `search.embeddings.api_key = { env = "ENV_NAME"; }` or
  `{ file = "/absolute/runtime/key"; }`;
- a `sync.postgres_url` password supplied through `\${ENV_NAME}` or
  `\${file:/absolute/runtime/password}`.

References cannot point into the Nix store. Kit requires embedding credential
files to be user-owned regular mode-0600 files and rejects symlinks. Environment
references use the caller's environment for the CLI; `services.roborev.environmentFiles`
supplies the daemon's environment. `programs.roborev.environment` supplies shared
**non-secret** values to both. Arbitrary freeform settings, injected agent config,
shell hook commands and harness base settings cannot be recognized as secret-free
by their shape: keep credentials out of those store-backed values.

Config-only credentials have a complete-file route:

```nix
programs.roborev = {
  enable = true;
  package = roborevPackage;
  runtimeConfigFile = "/run/user/1000/roborev-private.toml";
  extraPackages = [approvedAgentPackage];
};
services.roborev.enable = true; # Optional; program-only operation also works.
```

The secret owner provisions **the entire effective TOML**, including agent paths,
worker settings and any authentication, Discord/webhook URLs or embedded agent
credentials. `runtimeConfigFile` is mutually exclusive with `settings` and
`agentCommands`; `defaultAgent` is not injected into this file. There is no
additional merge or environment-based daemon-auth override. The managed
`dataDir/config.toml` points to that external file, so CLI, hooks, stdio MCP and
daemon use the same global configuration and authentication key. Its contents are
never read during Nix evaluation or builds.

The file must exist before activation, be readable, owned by the invoking user,
mode 0600, and not be a symlink. Activation and every wrapped invocation check
this contract. The secret owner controls provisioning, removal and rotation;
replace a private mode-0600 temporary file atomically at the same path. Restart
the daemon after rotation, especially for `auth_key`: changing external contents
does not change the Nix generation or automatically restart the service. Clients
read the new file on their next invocation. Retain the old key until the daemon
restart is complete if availability during rotation is required.

Switching between generated and runtime-file modes uses Home Manager's managed
link transition. Existing unmanaged `config.toml` files cause activation to fail;
preserve and relocate them before opting in. State, databases and sessions stay
Roborev-owned. Dedicated resource limits are unnecessary: configure
`systemd.user.services.roborev.Service` through the standard HM options.

### Repository files

```nix
programs.roborev.repositories.example = {
  settings = {
    review_md_fallback = false;
    post_commit_review = "branch";
    post_commit_batch_size = 5;
    excluded_branches = ["scratch"];
    ci.reviews = { gemini = ["security" "design"]; };
  };
  # Explicit opt-in; omit to render only.
  target = "${config.home.homeDirectory}/projects/example/.roborev.toml";
};
```

`configFile` exposes the rendered artifact for composition. `target` authorizes
Home Manager to own that exact file below the user's home; it does not authorize
Git staging, enrollment or hook installation. No file is overwritten with `force`.
Keep a tracked project-owned `.roborev.toml` under the project's existing owner,
or explicitly migrate it before selecting an HM target. Sensitive repository
webhook settings belong in an externally provisioned private repository file,
rather than the non-secret renderer. Global-only keys are rejected in repository
settings.

### Native harness hooks, skills and MCP clients

`integrations.<profile>` supports `claude`, `codex`, `copilot`, `cursor`, `droid`,
`gemini`, `hermes`, `qwen` and `grok`. These are harness profiles, not review agent
names. Upstream has no OpenCode native hook profile. OpenCode can consume the
wrapped executable through its existing MCP/configuration owner.

```nix
programs.roborev.integrations.gemini = {
  hooks = {
    enable = true;
    timeoutSeconds = 10;
    settings = { /* existing declarative Gemini settings */ };
    target = "${config.home.homeDirectory}/.gemini/settings.json";
  };
  mcp = {
    enable = true;
    target = "${config.home.homeDirectory}/.gemini/settings.json";
  };
  skills.enable = true; # Render only; select skills.target to install.
};
```

Rendering runs upstream `agent-hook dump`, `mcp install` and `skills install`
inside a disposable build home, never against live configuration. The resulting
`integrations.<profile>.package` has `hooks`, `mcp` and/or `skills` artifacts.
`rendererPackage` defaults to `package`; cross consumers supply a build-platform
renderer from the same pinned upstream version.
Explicit targets opt into normal HM ownership and collision checking. Supply
existing non-secret declarative settings in `hooks.settings` or `mcp.settings`
to preserve them in the generated document. Live unmanaged files are not merged
at activation. Skills use the upstream variant and include Codex metadata only
for Codex; use a dedicated `skills.target` directory to avoid unrelated skills.

Gemini, Qwen and Hermes share hook/MCP configuration formats and upstream files.
When both are enabled, put base settings in `hooks.settings`; both artifacts
contain the combined document and may select the same target. Other profiles
require separate hook/MCP targets. HTTP MCP uses `mcp.transport = "http"` and an
existing credential-free `mcp.url` ending in `/mcp`; native hooks select that
daemon too. For stdio, `server` optionally selects the daemon and `mcp.url` must
be omitted. Rendering a client does not enable the daemon's MCP listener: set
global `settings.mcp.enabled` or its equivalent in the private runtime TOML.

### Coverage gate

The existing `checks.<system>.roborev-module` gate now compares field coverage
against the packaged Go structs, including embedded GitHub App and locked Kit
embedding/secret-reference fields, and emits `configuration-paths.json` with
nested, dynamic-map and list-member paths. It also checks TOML omission semantics,
all nine native harness artifacts, private-file rejection and atomic replacement,
and real offline CLI/daemon authentication. `roborev-unit` retains the generated
systemd lifecycle and Home Manager switch coverage. Fork-side CI must qualify
the exact source before consumer pin promotion.

## Rust interfaces

Enable `canix-toolbelt`'s `review` feature. `RoborevUnix` implements the bounded
daemon transport with a mandatory controller checkout verifier.
`RoborevHttp` shares that transport with `RoborevLocal`, which preserves the
standalone client's direct-daemon journals and captured working-tree recovery.
Local committed/working-tree scopes remain advisory; their actual compared base
is retained and cannot qualify a different PR through dedicated-App publication.
`dispatch_roborev_once` records UNKNOWN before enqueue and reconciles the original
request-exclusive job without replay. Persisted receipts must attest the exact
full `target-tip..head` comparison and complete findings; a Markdown verdict is
insufficient. `RoborevGitHub` authenticates receipts against the dedicated policy
App and fresh forge facts. Policy loading is explicit and never falls back to an
unselected provider.

`RoborevDispatch.expected_files` is the independently verified full comparison
census. The preparation callback returns `(checkout, agent, expected_files, runner)`;
the runner verifies the checkout and census on every dispatch. A complete receipt
requires persisted coverage with no excluded files and exactly that reviewed count.
`RoborevRunner::enqueue` returns `RoborevJobIdentity { id, uuid }`; dispatch journals
retain both identities and correlate every listing and saved review against them.
`RoborevReceipt::from_saved_review` accepts the frozen dispatch, policy, original
job identity and persisted review. Dispatch journal version 2 preserves UNKNOWN
recovery; older numeric-ID-only journals remain intact and require reconciliation
with their original implementation rather than acquiring UUID authority implicitly.

Version 0.3 adds fields to `Review` and binds `GitHub::publish_check` to an explicit
policy. Downstream struct literals must initialize `provider_job_id`,
`provider_review_id` and `reviewed_base`; older serialized records still decode.
Review history upgrades in place, retaining lock inodes and earlier request IDs.
An intact legacy attempt may resume, but incomplete historical state cannot grant
a new comparison's dispatch.

The independent `canix-toolbelt-roborev-worker` crate owns Linux preparation,
direct adapter authentication, admission, execution fencing, offline isolation and
live verification. Its `roborev-worker` executable implements the legacy private
helper arguments. See its packaged README for persistent compatibility and native
qualification inputs. The portable library retains `forbid(unsafe_code)`.
Production reservations require original `Admission` custody; standalone fence
registration/loading/reservation are available only to native fixture features.

## Qualification boundary

Package/module tests, native Linux worker fixtures and real-daemon protocol tests
are separate evidence. Offline fixtures do not qualify a production credential
broker, root-owned custody or live provider/forge execution. A consumer adopts
registry APIs only after verifying the released artifact, and Nix interfaces only
after exact-source producer CI passes. Existing UNKNOWN attempts are never
reset or replayed during migration.

The required native-worker CI job prepares namespace admission on its disposable
GitHub-hosted Ubuntu runner before running Nix-store Bubblewrap. This prerequisite
does not change the worker's confinement policy or skip isolation tests.
