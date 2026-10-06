# OpenCode project environments

Harbor owns the generic environment engine and plugin APIs in `harbor-llm`.
Toolbelt owns the Canix-fork runtime patch, architecture-specific OOM policy,
legacy capability aliases and the mixed-version preparation-lock transition.
Consumers supply project membership, roots, backend endpoints and opt-in.
Infernix owns model/provider configuration; this adapter does not serve models.

## V2 integration

Import `homeModules.opencode-environment` and supply the generic payload:

```nix
canix-toolbelt.opencodeEnvironment = {
  enable = true;
  package = inputs.harbor-llm.packages.${pkgs.stdenv.hostPlatform.system}.default;
  options = {
    roots = [ "/workspaces/example" ];
    serverURL = "http://127.0.0.1:4096";
    opencode = "${compatibleOpencode}/bin/opencode";
    direnvApproval = "manual";
  };
};
```

The module supplies immutable tool paths and a native V2 plugin entry.
Its wrapper delegates to Harbor without implementing a second environment
engine. Keep Harbor's direct OpenCode entrypoint disabled when using this
integration so preparation is installed once.

The wrapper uses the historical `harbor-canix-llm` preparation-lock directory
under the private runtime directory (or the same home-cache fallback). This
keeps old and new processes on identical persistent anchor inodes. Override
`options.preparationLockDirectory` only after old holders have drained, or to
use an already-shared explicit directory. Never delete anchors to release locks.
Explicit consistency locks for declaration/lock promotion remain consumer data.

## Legacy runtime

`lib.opencodeEnvironment.patchLegacy package` applies the downstream patch to
the real OpenCode derivation. It implements Harbor's `harborLlm: 1` /
`harborLlmReplace` contract, retains the previous handshake for mixed-version
consumers, and restores the parent `OPENCODE_TOOL_OOM_SCORE_ADJ` after replacement.
Both package capability markers are retained; wrappers must preserve them.
The V2 Bash path is deliberately rejected after command permission checks.

The patch targets the legacy Canix fork's `b79c099b61ed0e67b5020844367cb6c1ba61c1eb`
`plugin/shell-environment.ts` API,
not the current native V2 backend. Verify applicability against the exact
selected legacy source before using it. Toolbelt does not select that source
or silently patch a V2 executable.

```sh
node runtime/opencode-environment/check-legacy.mjs /path/to/legacy-opencode-source
node --test runtime/opencode-environment/*.test.mjs
```

The legacy check works on temporary copies, checks applicability, verifies
command permission ordering and replacement-branch ordering, and checks
OOM preservation plus both capability handshakes. It makes no provider requests.
Generic resolver, approval, isolation and lifecycle tests remain in Harbor.
