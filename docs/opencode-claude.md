# Claude subscriptions in OpenCode V2

`homeModules.opencode-claude` composes the native `opencode-with-claude`
plugin, a per-user Meridian service and `homeModules.opencode-jev`.
The request path is OpenCode → Jev → Meridian → Claude Agent SDK.
Fleetix resolves both HTTP loopback endpoints on the selected host.

Import the module and enable OpenCode and Jev:

```nix
{
  imports = [inputs.canix-toolbelt.homeModules.opencode-claude];
  programs.opencode.enable = true;
  canix-toolbelt.opencodeJev = {
    enable = true;
    package = jevPackage;
    credentialFile = "%t/agenix/typesafe";
  };
  canix-toolbelt.opencodeClaude = {
    enable = true;
    topology = fleetTopology;
    hostName = "example";
    meridianEndpoint = "meridian-example-user";
    jevEndpoint = "jev-claude-example-user";
  };
}
```

Declare distinct ports for different users in Fleetix. Its endpoint resolver
rejects another host's loopback endpoint. Jev validates unique gateway ports.
The OpenCode service should require `opencodeClaude.requiredUnits`, want
`opencodeJev.units`, and start after both. Consumers set their own systemd
resource limits on `meridian-opencode.service`.

Each service reads its user's `~/.claude` login directory. Run `claude auth
login` as that user, then inspect `claude auth status` and Meridian's `/health`.
If `claudeConfigDirectory` is set, use the same `CLAUDE_CONFIG_DIR` when logging in.
No subscription credentials enter Nix configuration or the store. Imported
Anthropic API/router environment variables are removed from the service.
Meridian runs in passthrough mode: OpenCode retains tool execution.

The package pins `opencode-with-claude` 1.11.1, Meridian 1.79.0 and their npm
dependency closure. It selects a Nix-packaged Claude executable explicitly with
`MERIDIAN_CLAUDE_PATH`, disables npm install scripts and supplies `server.js`
for V2's local-directory loader. The upstream plugin already implements V2's
`setup(ctx)` API.

Toolbelt's flake package set permits only `claude-code` through an explicit
unfree-package predicate. Home Manager consumers supply their own package policy.

Until upstream supports managed endpoints, the package carries a small,
drift-checked patch adding `options.externalBaseURL`. External mode never
starts or stops Meridian. It registers per-request directory hooks using the
session's location, preserving project context after upstream prompt scrubbing.
The unconfigured embedded mode retains upstream behavior. Retire the patch
when a qualified upstream release provides both external-endpoint lifecycle
and per-session directory propagation.

Package install checks exercise real upstream V2 hooks, beta-header removal,
auxiliary requests, session affinity and independent cleanup. The module
evaluation check verifies topology, plugin ordering, Jev routes and credentials;
the Jev runtime check covers streaming bodies, query parameters and cancellation.
Live subscription inference remains a separate acceptance step after login and
activation.
