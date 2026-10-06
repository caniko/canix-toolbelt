# Muse Code Subscription Plugin

Operator instructions: [Muse Code](../../docs/opencode-muse-code.md).

This dependency-free ESM plugin uses the existing OpenCode auth/config/fetch
interfaces. OpenCode itself provides the pinned `@ai-sdk/openai` Responses
transport. The plugin adds no npm runtime dependencies and makes no changes to
OMP, `meta`, Zen, default models, or OmniRoute routing.

## Pinned Sources

Inspected on 2026-09-10:

- [oh-my-pi PR #10677](https://github.com/can1357/oh-my-pi/pull/10677), head
  `6785d70d53e2bc10aad259a532f6b5255275991e` in `eggpeat/oh-my-pi`.
  Protocol and capability adaptation retains its MIT license in `LICENSE`.
- OMP auth rule: `packages/catalog/src/compat/rules/auth/muse-code.kdl`.
- OMP device engine and poller: `packages/ai/src/registry/engine/device-code.ts`,
  `packages/ai/src/registry/oauth/device-code.ts`.
- OMP key exchange and transport: `packages/ai/src/registry/oauth/muse-code.ts`,
  `packages/ai/src/registry/muse-code.ts`.
- OMP discovery and capabilities: `packages/catalog/src/provider-models/openai-compat.ts`,
  `packages/catalog/src/compat/rules/providers/muse-code.kdl`.
- OMP usage: `packages/ai/src/usage/muse-code.ts` and its tests.
- OMP regression evidence: `packages/ai/test/registry/oauth/muse-code.test.ts`,
  `packages/ai/test/muse-code-provider.test.ts`,
  `packages/catalog/test/meta-provider.test.ts`,
  `packages/ai/test/muse-code-usage.test.ts`.
- Requested upstream OpenCode interface:
  [plugin/src/index.ts at b3f1a96c6dd7adeb28b36dd11add1998fc84d67b](https://github.com/anomalyco/opencode/blob/b3f1a96c6dd7adeb28b36dd11add1998fc84d67b/packages/plugin/src/index.ts).
- Actual installed and packaged runtime: Caniko OpenCode
  `21105065b9e74d80f4f1c85b082e546ec9254791`, version `1.18.29+2110506`,
   used by the original integration. Legacy patches target this revision;
   consumers supply their own compatible OpenCode input.
- [OpenCode plugins documentation](https://opencode.ai/docs/plugins/) and
  [provider documentation](https://opencode.ai/docs/providers/), retrieved
  2026-09-10. These are live documentation URLs, not immutable revisions.

## Compatibility Decisions

`visibility.patch` is a provider-scoped fix for an observed runtime failure:
`Provider.list()` drops providers with no models, and the `/provider` handler
only combines connected providers with models.dev. Consequently the CLI can
recognize a plugin's auth hook while the TUI `/connect` cannot see it. The patch
adds only the configured `muse-code` row and excludes empty model maps from
default-model selection. `transform.patch` shares the Meta Responses reasoning
parser with `muse-code` (variant order, `reasoningSummary`, encrypted reasoning
include). `packaged.test.mjs` fails on the original binary and
passes on the patched binary. `package.nix` uses `overrideAttrs.patches` on the
existing pinned source build and its unchanged, hash-pinned dependencies.

The runtime runs `provider.models` before adding config-defined providers to
its database. Instead of expanding the core patch, discovery runs in the
plugin's `config` hook. The SDK exposes credential writes but not reads there;
the plugin reads only the `muse-code` entry from OpenCode's documented
`$XDG_DATA_HOME/opencode/auth.json` storage (default `~/.local/share`). It also
respects OpenCode's existing `OPENCODE_AUTH_CONTENT` override. It never writes
that file itself or copies another provider's credentials.

The plugin uses OpenCode's supported OAuth device flow (`method: "auto"`,
argument-free callback), the same interface as the built-in device-code
providers. Approving in the browser completes the login automatically; nothing
is pasted back into OpenCode. Polling starts when OpenCode invokes the callback
and is bounded by the returned device expiry. OpenCode's OAuth callback carries
no AbortSignal: a new login or plugin disposal cancels the attempt, and CLI
Ctrl+C terminates the process. Closing the TUI dialog does not cancel an
already-running server-side poll; do not treat Esc as an immediate network
cancellation signal.

The TUI disposes and reloads its instance after successful authentication, so
discovery runs again. CLI login changes persistent credentials; restart the
shared backend afterwards. Inference re-reads the credential through the auth
loader on every request and rejects logout or account/key changes before sending.
No stale model list authorizes another account.

Discovery uses only the subscription key at `https://api.meta.ai/v1/models`.
An authoritative empty list and unavailable discovery have different status
states. Neither produces fallback models. Only returned IDs with exact known
metadata from the pinned reference are selectable. Unknown IDs are counted as
unsupported, not assigned guessed token limits or reasoning variants. Contributor
models are never selected automatically; the reference notes their prompts may
be used for training. Display labels are short (`Spark 1.3`, not the raw API
ID); API IDs are unchanged.

Reasoning variants have one owner, not two: `transform.patch` adds a shared
`openaiResponsesVariants` builder in the pinned runtime's `transform.ts` used
by both the Meta API-key provider and `muse-code` (same `reasoningSummary:
auto` and encrypted-reasoning include). Effort order is weakest to strongest —
`minimal, low, medium, high, xhigh`, with `max` last only on `muse-spark-1.3`
(`MUSE_EFFORTS`; `meta` keeps its own list including `none`). Without the
shared handling, the generic provider merge prepends `low/medium/high` first
and the selector shows the wrong order. The plugin supplies variant keys only
(same order, via one `museVariants` helper) for the merge and for effort
validation in `serialize()`; it owns no variant settings and no longer
duplicates store/include handling in its `chat.params` hook.

Two auth-dedup rules: each login authorization polls at most once (OpenCode
invokes the automatic callback a single time), and key exchanges deduplicate
onboarding and quota lookups separately — only the onboard request returns an
inference key — while rate limiting stays account-scoped.

Inference is OpenAI Responses at `https://api.meta.ai/v1/responses`, not Chat
Completions. The fetch wrapper rejects all other destinations and redirects,
replaces rather than merges credential headers, caps output tokens, requests
encrypted reasoning, keeps function calls/results and reasoning items intact,
and rejects custom/freeform/hosted tools. Account tokens go only to the verified
subscription-key endpoint. No refresh-token grant or per-request key exchange
exists. Key calls and quota requests are deduplicated within the server process;
quota successes and failures are cached for five minutes. HTTP Retry-After is
honored without an alternate account or provider.

Quota reporting is the `muse_code_status` plugin tool. It emits allowlisted
percentages/timestamps only, no account identity, keys, raw response, or payment
URL. OpenCode's token-cost counters remain zero for this provider because they
are not actual subscription charges, not because the subscription is free.

## Checks

```sh
node --test runtime/opencode-muse-code/protocol.test.mjs runtime/opencode-muse-code/v2.test.mjs
```

The toolbelt flake checks run deterministic protocol and V2 adapter tests.
The exported `flakeModules.opencode-muse-code` additionally registers the legacy
packaged check when the consumer supplies `inputs.opencode`: TypeScript `checkJs`
using that runtime's compiler and Node typings, and the compiled OpenCode against
a fake remote service in an isolated home. The packaged test disables unrelated
npm network installation via npm's offline mode. It does not mock OpenCode's
auth storage, provider registration, AI SDK, tool execution, session persistence,
or reasoning serialization. All remote credentials in tests are synthetic.

These checks are not live Meta service verification. Browser approval and a
subscription-backed request remain operator steps.

## Upstream v2 Implementation

The upstream PR slice lives on the `muse-code-subscription` branch of the
opencode fork (based on `upstream/v2`), referencing
[anomalyco/opencode#41551](https://github.com/anomalyco/opencode/issues/41551):
`packages/ai/src/providers/muse-code.ts` (Responses route on the shared
MetaResponses protocol, explicit subscription bearer, single-owner model
policy and auth helpers), `packages/ai/test/provider/muse-code.test.ts`,
`packages/core/src/plugin/provider/muse-code.ts` (device-code OAuth through
the shared ai helpers, subscription-gated catalog models, single-flight key
exchange with Retry-After backoff, no refresh schedule), plus provider
registration and the `muse-code` provider ID. Validated locally with a pinned
bun: ai/core typechecks, 15/15 muse-code ai tests (including a Responses
transport round trip), neighboring meta/models suites green, prettier clean.
The TUI still renders all OAuth callback failures as “Invalid code”, hiding
subscription and billing errors — that display fix is tracked as upstream
follow-up, along with live browser-approved verification.
