import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { decode, encode, negotiate, type Message, ProtocolError } from "./protocol.ts";
import { schemaDigest } from "./types.ts";

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
