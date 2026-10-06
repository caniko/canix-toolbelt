# OpenCode Muse Code adapter

canix-toolbelt owns the reusable Muse Code subscription plugin, protocol,
compatibility patches and tests. Importing the module is inert; provider use
requires an explicit opt-in by the consuming Home Manager configuration.
It has no fleet, host, gateway, agent-routing or secret-registry dependencies.

## Home Manager

Import this module alongside the consumer's OpenCode Home Manager module:

```nix
{
  imports = [inputs.canix-toolbelt.homeModules.opencode-muse-code];
  programs.opencode.enable = true;
  canix-toolbelt.opencodeMuseCode.enable = true;
}
```

The default `apiVersion = "v2"` emits a native `plugins` entry using `server.js`.
Credentials remain in OpenCode's native integration storage. An optional
`legacyAuthFile = "/absolute/private/auth.json"` exposes an explicit import
method; it does not automatically copy credentials. Never supply a Nix store
credential path or embed tokens in configuration.

Legacy consumers can set `apiVersion = "v1"` to emit the `index.mjs` entrypoint.
The legacy package adapter at `runtime/opencode-muse-code/package.nix` accepts
a compatible source-built OpenCode derivation and applies the preserved V1
visibility/reasoning patches. Those patches target the historical runtime
listed in the [implementation notes](../runtime/opencode-muse-code/README.md);
they are not required by the native V2 adapter.

The module does not choose a default model, define agents, install a gateway,
or initiate login. Use the native `/connect` UI only after intentionally
enabling the adapter in the consuming configuration. Complete device approval
yourself, then select an account-discovered model in `/models`.

Discovery and inference use the subscription endpoint only. Unknown models
remain unavailable; there is no Meta PAYG, alternate-account or alternate-provider
fallback. Subscription eligibility and live authorization are not established
by fixture tests.

## Validation

From the toolbelt checkout, run native tests in its approved environment:

```sh
node --test runtime/opencode-muse-code/protocol.test.mjs runtime/opencode-muse-code/v2.test.mjs
treefmt --fail-on-change
```

The toolbelt flake registers `opencode-muse-code-module`,
`opencode-muse-code-protocol` and `opencode-muse-code-v2` checks.
Consumers can import `inputs.canix-toolbelt.flakeModules.opencode-muse-code`
to register the same checks. When that consumer also supplies `inputs.opencode`,
the module exposes the legacy `packages.opencode-muse-code` adapter and
`checks.muse-code-subscription`, preserving the compiler check and compiled-runtime
test against a fake remote service. Native V2 module evaluation does not require
an OpenCode flake input.

Disabling `canix-toolbelt.opencodeMuseCode.enable` removes the plugin declaration.
Use the consumer's provider policy to deny `muse-code` and `meta` when all Muse
usage must be deactivated, including support bundled by the upstream runtime.
V2 uses `experimental.policies` with `action = "provider.use"` and
`effect = "deny"`; the V1 `disabled_providers` list is not a V2 policy.
Credential deletion is a separate native disconnect operation.
