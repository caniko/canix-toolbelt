import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { decode, encode, negotiate, type Message, ProtocolError } from "./protocol.ts";
import { schema, schemaDigest } from "./types.ts";

const fixtures = JSON.parse(readFileSync(new URL("../../tests/execution-protocol-fixtures.json", import.meta.url), "utf8"));
const encoder = new TextEncoder();

function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (typeof value === "object" && value !== null) return `{${Object.keys(value).sort().map(key => `${JSON.stringify(key)}:${canonical(value[key])}`).join(",")}}`;
  return JSON.stringify(value);
}

function locate(value, path) {
  const fields = path.split(".");
  const field = fields.pop();
  return [fields.reduce((parent, key) => parent[key], value), field];
}

for (const fixture of fixtures.cases) {
  test(fixture.name, () => {
    const value = fixture.raw === undefined ? structuredClone(fixtures.messages[fixture.base]) : null;
    for (const [path, replacement] of Object.entries(fixture.changes ?? {})) {
      const [parent, field] = locate(value, path);
      parent[field] = replacement;
    }
    if (fixture.remove) {
      const [parent, field] = locate(value, fixture.remove);
      delete parent[field];
    }
    const bytes = encoder.encode(fixture.raw ?? canonical(value));
    if (fixture.accepted) assert.deepEqual(encode(decode(bytes)), bytes);
    else {
      assert.throws(() => decode(bytes), ProtocolError);
      assert.throws(() => decode(bytes), { code: fixture.code ?? fixtures.rejectionCodeDefault });
    }
  });
}

test("UTF-8 bounds and malformed frames", () => {
  for (const requestId of ["x".repeat(257), "🧪".repeat(65)]) {
    const value = structuredClone(fixtures.messages.describe);
    value.requestId = requestId;
    assert.throws(() => decode(encoder.encode(canonical(value))), ProtocolError);
  }
  const value = structuredClone(fixtures.messages.describe);
  value.requestId = "🧪".repeat(64);
  decode(encoder.encode(canonical(value)));
  assert.throws(() => decode(new Uint8Array(65537)), ProtocolError);
  assert.throws(() => decode(new Uint8Array([255])), ProtocolError);
  assert.throws(() => decode(encoder.encode("[".repeat(65536))), ProtocolError);
});

function message(name: string): Message {
  const value = structuredClone(fixtures.messages[name]);
  if (name === "describe") value.body.expectedImplementation.schemaDigest = schemaDigest;
  if (name === "capabilities") value.body.implementation.schemaDigest = schemaDigest;
  return decode(encoder.encode(canonical(value)));
}

test("discovery binds correlation, implementation, and zero-effect recovery", () => {
  const request = message("describe");
  const response = message("capabilities");
  assert.equal(response.body.kind, "capabilities");
  if (response.body.kind !== "capabilities") throw new Error("fixture");
  negotiate(request, response, ["retained-delivery"]);
  response.requestId = "another-request";
  assert.throws(() => negotiate(request, response, []), { code: "transport-failed" });
  response.requestId = request.requestId;
  const wrong = structuredClone(request);
  if (wrong.body.kind !== "describe") throw new Error("fixture");
  wrong.body.expectedImplementation.engineRevision = "1".repeat(40);
  assert.throws(() => negotiate(wrong, response, []), { code: "custody-lost" });
  response.body.capabilities = response.body.capabilities.filter(c => c !== "zero-effect-reopen");
  assert.throws(() => negotiate(request, response, []), { code: "unsupported-capability" });
});

test("unsupported macOS worker rejects even an advertised capability", () => {
  const request = message("describe");
  const response = message("capabilities");
  if (request.body.kind !== "describe" || response.body.kind !== "capabilities") throw new Error("fixture");
  request.body.requiredProfile = "macos-untrusted-review";
  response.body.profiles.push("macos-untrusted-review");
  assert.throws(() => negotiate(request, response, []), { code: "unsupported-profile" });
});

test("every implementation field and mandatory guarantee is bound", () => {
  const request = message("describe");
  const response = message("capabilities");
  if (request.body.kind !== "describe" || response.body.kind !== "capabilities") throw new Error("fixture");
  for (const field of Object.keys(response.body.implementation)) {
    const wrong = structuredClone(response);
    if (wrong.body.kind !== "capabilities") throw new Error("fixture");
    wrong.body.implementation[field] = "a".repeat(field === "engineRevision" ? 40 : 64);
    assert.throws(() => negotiate(request, wrong, []), { code: "custody-lost" });
  }
  const wrongRequest = structuredClone(request);
  const wrongResponse = structuredClone(response);
  if (wrongRequest.body.kind !== "describe" || wrongResponse.body.kind !== "capabilities") throw new Error("fixture");
  wrongRequest.body.expectedImplementation.schemaDigest = "a".repeat(64);
  wrongResponse.body.implementation.schemaDigest = "a".repeat(64);
  assert.throws(() => negotiate(wrongRequest, wrongResponse, []), { code: "custody-lost" });
  for (const missing of response.body.capabilities) {
    const wrong = structuredClone(response);
    if (wrong.body.kind !== "capabilities") throw new Error("fixture");
    wrong.body.capabilities = wrong.body.capabilities.filter(capability => capability !== missing);
    assert.throws(() => negotiate(request, wrong, ["retained-delivery"]), { code: "unsupported-capability" });
  }
  response.body.profiles = [];
  assert.throws(() => negotiate(request, response, []), { code: "unsupported-profile" });
});

test("sparse protocol arrays cannot be encoded or negotiated", () => {
  const request = message("describe");
  for (const field of ["profiles", "capabilities"] as const) {
    for (const gap of ["deleted", "extended", "inherited"] as const) {
      const response = message("capabilities");
      if (response.body.kind !== "capabilities") throw new Error("fixture");
      const items = response.body[field];
      const index = items.length - 1;
      if (gap === "extended") items.length += 1;
      else {
        const removed = items[index];
        delete items[index];
        if (gap === "inherited") {
          const prototype = Object.create(Array.prototype);
          Object.defineProperty(prototype, index, { value: removed });
          Object.setPrototypeOf(items, prototype);
        }
      }
      assert.throws(() => encode(response), { code: "invalid-frame" }, `${field}: ${gap}`);
      assert.throws(() => negotiate(request, response, []), { code: "invalid-frame" }, `${field}: ${gap}`);
    }
  }
});

test("exported schema metadata is recursively immutable", () => {
  const source = JSON.parse(readFileSync(new URL("./protocol.json", import.meta.url), "utf8"));
  assert.deepEqual(schema, source);
  function assertFrozen(value: unknown): void {
    if (typeof value !== "object" || value === null) return;
    assert.equal(Object.isFrozen(value), true);
    for (const child of Object.values(value)) assertFrozen(child);
  }
  assertFrozen(schema);
  assert.equal(Reflect.set(schema, "maxFrameBytes", 131072), false);
  assert.equal(Reflect.set(schema.definitions.Id, "maxBytes", 1024), false);
  assert.equal(Reflect.set(schema.definitions.Profile.enum, "0", "unknown-profile"), false);
  assert.equal(Reflect.deleteProperty(schema.definitions.Message.fields, "schemaVersion"), false);
  assert.equal(Reflect.set(schema.definitions.Capabilities.fields.profiles, "maxItems", 5), false);
  assert.deepEqual(schema, source);
});

test("schema mutation cannot relax identity validation", () => {
  const original = schema.definitions.Id.maxBytes;
  try {
    Reflect.set(schema.definitions.Id, "maxBytes", 1024);
    const value = message("describe");
    value.requestId = "x".repeat(original + 1);
    assert.throws(() => encode(value), { code: "invalid-frame" });
    assert.throws(() => decode(encoder.encode(canonical(value))), { code: "invalid-frame" });
  } finally {
    // Restore a mutable baseline during RED without changing frozen metadata.
    if (!Object.isFrozen(schema.definitions.Id)) Reflect.set(schema.definitions.Id, "maxBytes", original);
  }
});

test("encoding validates and emits the same accessor-backed identity", () => {
  const value = message("describe");
  let reads = 0;
  Object.defineProperty(value, "requestId", {
    enumerable: true,
    get: () => ++reads === 1 ? "ok" : "bad\nvalue",
  });
  assert.equal(decode(encode(value)).requestId, "ok");
});

test("encoding snapshots nested accessors before validation", () => {
  const value = message("describe");
  if (value.body.kind !== "describe") throw new Error("fixture");
  const implementation = value.body.expectedImplementation;
  const digest = implementation.engineDigest;
  let reads = 0;
  Object.defineProperty(implementation, "engineDigest", {
    enumerable: true,
    get: () => ++reads === 1 ? digest : "invalid-digest",
  });
  const decoded = decode(encode(value));
  if (decoded.body.kind !== "describe") throw new Error("fixture");
  assert.equal(decoded.body.expectedImplementation.engineDigest, digest);
});

test("encoding snapshots array accessors before validation", () => {
  const value = message("capabilities");
  if (value.body.kind !== "capabilities") throw new Error("fixture");
  let reads = 0;
  Object.defineProperty(value.body.profiles, 0, {
    enumerable: true,
    get: () => ++reads === 1 ? "linux-untrusted-review" : "unknown-profile",
  });
  const decoded = decode(encode(value));
  if (decoded.body.kind !== "capabilities") throw new Error("fixture");
  assert.equal(decoded.body.profiles[0], "linux-untrusted-review");
});

test("uncapturable messages produce redacted protocol errors", () => {
  const throwing = message("describe");
  Object.defineProperty(throwing, "requestId", {
    enumerable: true,
    get: () => { throw new Error("synthetic-private-getter-value"); },
  });
  for (const value of [throwing, new Proxy(message("describe"), {})]) {
    for (const operation of [() => encode(value), () => negotiate(value, message("capabilities"), [])]) {
      assert.throws(operation, error => error instanceof ProtocolError
        && error.code === "invalid-frame"
        && error.message === "execution protocol rejected: invalid-frame"
        && error.cause === undefined);
    }
  }
});

for (const binding of ["correlation", "implementation", "profile", "capabilities"] as const) {
  test(`discovery checks the captured ${binding} rather than rereading accessors`, () => {
    const request = message("describe");
    const response = message("capabilities");
    if (request.body.kind !== "describe" || response.body.kind !== "capabilities") throw new Error("fixture");
    let target: object;
    let key: string;
    let initial: unknown;
    let later: unknown;
    let code: string;
    switch (binding) {
      case "correlation":
        target = response; key = "requestId";
        initial = "mismatched-request"; later = request.requestId;
        code = "transport-failed";
        break;
      case "implementation":
        target = response.body.implementation; key = "engineDigest";
        initial = "a".repeat(64); later = response.body.implementation.engineDigest;
        code = "custody-lost";
        break;
      case "profile":
        target = request.body; key = "requiredProfile";
        initial = "macos-untrusted-review"; later = "linux-untrusted-review";
        code = "unsupported-profile";
        break;
      case "capabilities":
        target = response.body; key = "capabilities";
        initial = response.body.capabilities.filter(capability => capability !== "zero-effect-reopen");
        later = response.body.capabilities;
        code = "unsupported-capability";
        break;
    }
    let reads = 0;
    Object.defineProperty(target, key, {
      enumerable: true,
      get: () => ++reads === 1 ? initial : later,
    });
    assert.throws(() => negotiate(request, response, []), { code });
  });
}
