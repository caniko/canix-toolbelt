# Shared execution protocol

`execution_protocol` is a provider-free structural boundary for a supervised
engine bridge. Its schema, Rust types and TypeScript types ship in the Cargo
archive. The runtime custodian supplies authorization, process/credential custody
and qualification of the advertised guarantees.

## Wire contract

`runtime/execution/protocol.json` owns the version, field inventory, string
constraints, counter representation and structural frame limit. Generate both
type inventories with:

```sh
python3 scripts/generate-execution-protocol.py
python3 scripts/generate-execution-protocol.py --check
```

Generated TypeScript schema metadata is recursively frozen at initialization,
keeping exported metadata and validation constraints bound to the same digest.
JavaScript array inputs must contain an own element at every index; sparse arrays
and inherited replacements are rejected before encoding or negotiation.

`decode` accepts a single UTF-8 compact JSON document with recursively sorted
object keys and canonical string escapes. Duplicate keys, unknown fields,
alternate encodings, invalid UTF-8, unpaired surrogates and incompatible versions
are rejected. JSON numeric literals are unsigned 32-bit integers; decimal fractions,
exponents, negative values and larger literals are invalid frames before version
negotiation. All execution counters use strings. `encode` validates and emits that
same representation. Transport
delimiters are excluded from the schema's 65,536-byte frame limit; the future
transport must bound reads before decoding. Transport queues, deadlines,
outstanding requests, captured application output and deployment resource limits
require their own caller policy.

Opaque identifiers preserve UTF-8 bytes, prohibit Unicode control characters and
have a 256-byte limit. SHA-256 digests use 64 lowercase hexadecimal characters;
the engine revision uses the full 40-character Git revision. Generations and
counters are canonical decimal **strings** in the unsigned 64-bit range, so
JavaScript does not lose precision. Generations start at one. No identifier or
digest is itself an authorization grant.

`describe` binds the expected engine revision and executable digest, bridge
digest, exact schema digest, configuration digest and credential-reference
digest. The credential-reference digest identifies the scoped reference, never
secret credential bytes. `negotiate` checks request correlation, full
implementation equality, this library's actual schema digest, the requested
profile and mandatory authenticated-IPC/durable-intent/zero-effect-reopen/
descendant-cleanup guarantees. Required extra capabilities must also be present.
Changing executable, configuration or credential reference invalidates the match.

Negotiation evaluates custodian-supplied discovery evidence; an advertisement
alone establishes no platform support. macOS untrusted-review stays explicitly
unsupported even if a response advertises it. The managed profiles preserve
their application-specific local-host trust boundary.

Messages describe identities, observations, retained-output receipts and redacted
failures. Application semantic completion and retained-byte validation remain
with the caller. An observation marked `unknown`, an event gap, idle engine or
process exit does not authorize another effect or establish completion. The
protocol exposes no mutating engine operation in this slice.

## Conformance

Rust and TypeScript exercise `tests/execution-protocol-fixtures.json`, including
maximum counters, Unicode identities, duplicate keys, malformed messages and
unavailable guarantees:

```sh
cargo test --locked --no-default-features --test execution_protocol
node --test runtime/execution/protocol.test.ts
```

Use Node 24 or a compatible TypeScript-aware test runner. Formatting uses the
repository's `treefmt`. These checks do not invoke a provider or read application
sessions, secrets, journals or daemon state. Actual lifecycle, authenticated IPC,
the exact generated OpenCode V2 client and interrupted-state zero-effect reopening
remain separate implementation and qualification slices.
