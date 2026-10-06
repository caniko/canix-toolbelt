import { test } from "node:test";
import assert from "node:assert/strict";
import { BASE, ID, NAME, createProtocol, credential, modelsFromResponse, serialize } from "./protocol.mjs";
import plugin from "./index.mjs";

const device = { device_code: "test-device", user_code: "TEST-CODE", verification_uri: "https://auth.meta.com/device", interval: 1, expires_in: 60 };
const subscription = { api_key: "test-sub-key", user_id: "test-account", is_subs_active: true, subs_usage: { window: { used_percent: 12, window_duration_mins: 300 } } };
const auth = { type: "oauth", access: JSON.stringify({ accountToken: "test-account-token", apiKey: subscription.api_key }), accountId: subscription.user_id, refresh: "", expires: 8640000000000000 };
const modelList = { data: [{ id: "muse-spark-1.3" }, { id: "muse-spark-1.3-contributor" }] };
const models = modelsFromResponse(modelList).models;

function harness(options = {}) {
  let clock = 0;
  const requests = [];
  const waits = [];
  const tokens = [...(options.tokens ?? [{ access_token: "test-account-token" }])];
  const protocol = createProtocol({
    now: () => clock,
    wait: async (ms, signal) => { signal?.throwIfAborted(); waits.push(ms); clock += ms; },
    fetch: async (url, init) => {
      requests.push({ url, init });
      assert.equal(init.redirect, "error");
      assert.equal(new Headers(init.headers).get("x-api-version"), "1.0.0");
      if (url.endsWith("/authorization/")) return Response.json(options.device ?? device);
      if (url.endsWith("/token/")) {
        if ((options.tokenErrors ?? 0) > 0) { options.tokenErrors--; throw new Error("temporary network failure"); }
        const body = tokens.shift() ?? { error: "authorization_pending" };
        return Response.json(body, { status: body.error ? 400 : 200 });
      }
      if (url.endsWith("/key")) return options.keyResponse?.() ?? Response.json(options.key ?? subscription);
      if (url.endsWith("/models")) return Response.json(options.models ?? modelList);
      if (url.endsWith("/responses")) return options.inference?.(init) ?? new Response("data: [DONE]\n\n", { headers: { "Content-Type": "text/event-stream" } });
      throw new Error("unverified destination");
    },
  });
  return { protocol, requests, waits, advance: ms => { clock += ms; } };
}

test("provider visible before login; default models and other credentials/config are untouched", async () => {
  const previous = process.env.OPENCODE_AUTH_CONTENT;
  process.env.OPENCODE_AUTH_CONTENT = JSON.stringify({ meta: { type: "api", key: "test-payg" } });
  try {
    const hooks = await plugin();
    const config = { model: "other/model", provider: { meta: { name: "Meta" } } };
    await hooks.config(config);
    assert.equal(config.provider[ID].name, NAME);
    assert.equal(config.model, "other/model");
    assert.deepEqual(Object.keys(config.provider[ID].models), []);
    assert.deepEqual(config.provider[ID].whitelist, []);
    assert.equal(hooks.auth.provider, ID);
    assert.equal(hooks.auth.methods[0].type, "oauth");
    assert.equal(JSON.stringify(config).includes("test-payg"), false);
    assert.throws(() => credential({ type: "api", key: "test-payg" }), /API keys are not accepted/);
    await hooks.dispose();
  } finally {
    if (previous === undefined) delete process.env.OPENCODE_AUTH_CONTENT; else process.env.OPENCODE_AUTH_CONTENT = previous;
  }
});

test("device success completes automatically with exactly one exchange", async () => {
  const h = harness();
  const login = await h.protocol.authorize();
  assert.equal(login.method, "auto");
  assert.match(login.instructions, /TEST-CODE/);
  const a = await login.callback();
  assert.deepEqual(credential({ ...a, type: "oauth" }), credential(auth));
  assert.equal(a.refresh, "");
  assert.equal(a.expires, auth.expires);
  assert.equal(h.requests.filter(r => r.url.endsWith("/key")).length, 1);
  assert.deepEqual(JSON.parse(h.requests.at(-1).init.body), { onboard: true });
  const tokenBody = new URLSearchParams(h.requests[1].init.body);
  assert.equal(tokenBody.get("client_id"), "1031625952748946");
  assert.equal(tokenBody.get("grant_type"), "urn:ietf:params:oauth:grant-type:device_code");
});

test("authorize performs no token polling; polling starts on callback", async () => {
  const h = harness();
  const login = await h.protocol.authorize();
  assert.equal(h.requests.length, 1);
  assert.equal(login.callback.length, 0);
  await login.callback();
  assert.ok(h.requests.some(r => r.url.endsWith("/token/")));
});

test("aborted login cancels before exchange", async () => {
  const controller = new AbortController();
  const h = harness();
  const login = await h.protocol.authorize(controller.signal);
  controller.abort();
  await assert.rejects(login.callback(), /cancelled/);
  assert.equal(h.requests.length, 1);
});

for (const [error, expected] of [["access_denied", /denied/], ["expired_token", /expired/]]) {
  test(`device ${error}`, async () => {
    const h = harness({ tokens: [{ error }] });
    await assert.rejects((await h.protocol.authorize()).callback(), expected);
    assert.equal(h.requests.some(r => r.url.endsWith("/key")), false);
  });
}

test("pending and slow_down increase interval; local deadline is bounded", async () => {
  const h = harness({ tokens: [{ error: "authorization_pending" }, { error: "slow_down" }, { access_token: "test-account-token" }] });
  await (await h.protocol.authorize()).callback();
  assert.deepEqual(h.waits, [1000, 1000, 6000]);
  const expired = harness({ tokens: [], device: { ...device, expires_in: 2 } });
  await assert.rejects((await expired.protocol.authorize()).callback(), /expired/);
  assert.equal(expired.requests.length, 2);
});

for (const [name, key, message] of [
  ["inactive", { is_subs_active: false }, /inactive/],
  ["payment", { require_payment: true }, /billing/],
  ["payment URL is never echoed", { action_url: "https://evil.example/test-secret" }, /billing/],
  ["missing key", { user_id: "test" }, /invalid field/],
  ["missing identity", { api_key: "test" }, /invalid field/],
  ["malformed status", { ...subscription, is_subs_active: "yes" }, /malformed/],
]) test(`subscription ${name}`, async () => {
  const h = harness({ key });
  await assert.rejects((await h.protocol.authorize()).callback(), message);
});

test("rate limiting and malformed replies never expose response bodies", async () => {
  for (const [status, expected] of [[401, /expired/], [402, /payment/], [403, /denied/], [429, /rate limited/], [502, /HTTP 502/]]) {
    const h = harness({ keyResponse: () => new Response("test-account-token", { status, headers: { "Retry-After": "600" } }) });
    await assert.rejects((await h.protocol.authorize()).callback(), expected);
    if (status === 429) {
      await assert.rejects(h.protocol.status(auth), /rate limited/);
      assert.equal(h.requests.filter(r => r.url.endsWith("/key")).length, 1);
    }
  }
  const h = harness({ keyResponse: () => new Response("test-secret-not-json") });
  await assert.rejects((await h.protocol.authorize()).callback(), /malformed JSON/);
});

test("account discovery is authoritative; unknown revisions do not inherit guessed metadata", async () => {
  assert.equal(Object.keys(modelsFromResponse({ data: [] }).models).length, 0);
  assert.throws(() => modelsFromResponse({ models: [] }), /malformed/);
  const unknown = modelsFromResponse({ data: [{ id: "muse-spark-99" }] });
  assert.equal(unknown.entitledCount, 1);
  assert.equal(unknown.unsupportedCount, 1);
  assert.deepEqual(Object.keys(unknown.models), []);
  const h = harness();
  await h.protocol.discover(auth);
  assert.equal(new Headers(h.requests[0].init.headers).get("Authorization"), "Bearer test-sub-key");
  assert.equal(JSON.stringify(h.requests).includes("test-account-token"), false);
  const limited = modelsFromResponse({ data: [{ id: "muse-spark-1.3", context_length: 32768, max_completion_tokens: 4096 }] });
  assert.deepEqual(limited.models["muse-spark-1.3"].limit, { context: 32768, output: 4096 });
  assert.throws(() => modelsFromResponse({ data: [{ id: "muse-spark-1.3", max_completion_tokens: -1 }] }), /invalid token limits/);
});

test("display names are short; variant order is minimal to xhigh with max last", () => {
  const discovered = modelsFromResponse({ data: [
    { id: "muse-spark-1.1" }, { id: "muse-spark-1.2" }, { id: "muse-spark-1.2-contributor" },
    { id: "muse-spark-1.3" }, { id: "muse-spark-1.3-contributor" },
  ] });
  assert.equal(discovered.models["muse-spark-1.1"].name, "Spark 1.1");
  assert.equal(discovered.models["muse-spark-1.2"].name, "Spark 1.2");
  assert.equal(discovered.models["muse-spark-1.2-contributor"].name, "Spark 1.2 Contributor");
  assert.equal(discovered.models["muse-spark-1.3"].name, "Spark 1.3");
  assert.equal(discovered.models["muse-spark-1.3-contributor"].name, "Spark 1.3 Contributor");
  assert.deepEqual(Object.keys(discovered.models["muse-spark-1.2"].variants), ["minimal", "low", "medium", "high", "xhigh"]);
  assert.deepEqual(Object.keys(discovered.models["muse-spark-1.3"].variants), ["minimal", "low", "medium", "high", "xhigh", "max"]);
  // Variant settings are owned by transform.ts; the plugin supplies keys only.
  assert.deepEqual(discovered.models["muse-spark-1.2"].variants.low, {});
});

test("limits, effort variants and function tool serialization preserve reasoning and round trips", () => {
  const input = [
    { type: "reasoning", id: "rs_test", encrypted_content: "test-encrypted-reasoning", summary: [] },
    { type: "function_call", call_id: "call_test", name: "read", arguments: "{}" },
    { type: "function_call_output", call_id: "call_test", output: "test-result" },
  ];
  const body = { model: "muse-spark-1.3", reasoning: { effort: "max" }, input, max_output_tokens: 999999, tools: [{ type: "function", name: "read", parameters: { type: "object" } }] };
  const serialized = JSON.parse(serialize(JSON.stringify(body), models));
  assert.deepEqual(serialized.input, input);
  assert.equal(serialized.max_output_tokens, 131072);
  assert.equal(serialized.store, false);
  assert.deepEqual(serialized.include, ["reasoning.encrypted_content"]);
  assert.throws(() => serialize(JSON.stringify({ ...body, model: "muse-spark-1.3-contributor" }), models), /effort/);
  assert.throws(() => serialize(JSON.stringify({ ...body, tools: [{ type: "custom", name: "patch" }] }), models), /function tools only/);
  assert.throws(() => serialize(JSON.stringify({ ...body, model: "unentitled" }), models), /not in this account/);
});

test("stream passthrough, key reuse, logout, account changes and destination guard", async () => {
  const stream = "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"call_id\":\"call_test\"}}\n\n";
  const h = harness({ inference: () => new Response(stream) });
  let current = auth;
  const send = h.protocol.inference(async () => current, auth, models);
  const body = JSON.stringify({ model: "muse-spark-1.3", input: [] });
  assert.equal(await (await send(`${BASE}/responses`, { body })).text(), stream);
  await send(`${BASE}/responses`, { body });
  assert.equal(h.requests.length, 2);
  assert.equal(h.requests.every(r => r.url.endsWith("/responses")), true);
  assert.equal(JSON.stringify(h.requests).includes("test-account-token"), false);
  await assert.rejects(send("https://evil.example/responses", { body }), /unverified/);
  current = undefined;
  await assert.rejects(send(`${BASE}/responses`, { body }), /Connect Muse/);
  current = { ...auth, accountId: "other" };
  await assert.rejects(send(`${BASE}/responses`, { body }), /account changed/);
  assert.equal(h.requests.length, 2);
});

test("quota deduplicates concurrent requests, caches and redacts", async () => {
  const h = harness();
  const reports = await Promise.all([h.protocol.status(auth), h.protocol.status(auth)]);
  assert.equal(h.requests.length, 1);
  const cached = await h.protocol.status(auth);
  assert.equal(cached.cached, true);
  assert.equal(cached.windows[0].usedPercent, 12);
  assert.equal(JSON.stringify(reports).includes("test-account"), false);
  assert.equal(JSON.stringify(reports).includes("test-sub-key"), false);
  assert.deepEqual(JSON.parse(h.requests[0].init.body), {});
});

test("invalid verification destinations never become browser instructions", async () => {
  const h = harness({ device: { ...device, verification_uri: "https://evil.example" } });
  await assert.rejects(h.protocol.authorize(), /unverified/);
});

test("transient polling failure does not poison a later retry", async () => {
  const h = harness({ tokenErrors: 1 });
  const login = await h.protocol.authorize();
  await assert.rejects(login.callback(), /network request failed/);
  const result = await login.callback();
  assert.equal(result.type, "success");
  assert.equal(h.requests.filter(r => r.url.endsWith("/token/")).length, 2);
  assert.equal(h.requests.filter(r => r.url.endsWith("/key")).length, 1);
});

test("quota lookup never satisfies an onboarding exchange", async () => {
  let releaseQuota;
  const quotaGate = new Promise(resolve => { releaseQuota = resolve; });
  const bodies = [];
  const protocol = createProtocol({
    now: () => 0,
    wait: async () => {},
    fetch: async (url, init) => {
      if (url.endsWith("/authorization/")) return Response.json(device);
      if (url.endsWith("/token/")) return Response.json({ access_token: "test-account-token" });
      if (url.endsWith("/key")) {
        const body = JSON.parse(init.body);
        bodies.push(body);
        if (body.onboard === true) return Response.json({ api_key: "test-sub-key", user_id: "test-account", is_subs_active: true });
        await quotaGate;
        return Response.json({ is_subs_active: true, subs_usage: { window: { used_percent: 5 } } });
      }
      throw new Error("unverified destination");
    },
  });
  const status = protocol.status(auth);
  await new Promise(resolve => setImmediate(resolve));
  const login = await protocol.authorize();
  const result = await login.callback();
  assert.equal(result.type, "success");
  assert.deepEqual(bodies, [{}, { onboard: true }]);
  releaseQuota();
  const report = await status;
  assert.equal(report.windows[0].usedPercent, 5);
});

test("abort mid-poll cancels the login", async () => {
  const controller = new AbortController();
  const protocol = createProtocol({
    now: () => 0,
    wait: async () => {},
    fetch: async (url, init) => {
      if (url.endsWith("/authorization/")) return Response.json(device);
      if (url.endsWith("/token/")) {
        await new Promise((_resolve, reject) => init.signal?.addEventListener("abort", () => reject(new Error("aborted")), { once: true }));
      }
      throw new Error("unverified destination");
    },
  });
  const login = await protocol.authorize(controller.signal);
  const pending = login.callback();
  await new Promise(resolve => setImmediate(resolve));
  controller.abort();
  await assert.rejects(pending, /cancelled/);
});

test("quota-only responses need no fresh key or identity; timestamps are normalized", async () => {
  const h = harness({ key: { is_subs_active: true, subs_usage: { weekly: { used_percent: 30, resets_at: 1800000000 } } } });
  const status = await h.protocol.status(auth);
  assert.equal(status.windows[0].resetsAt, new Date(1800000000000).toISOString());
  assert.equal(h.requests.length, 1);
});

test("inference cancellation propagates the request signal without a fallback", async () => {
  const controller = new AbortController();
  const h = harness({ inference: init => new Promise((_resolve, reject) => {
    assert.equal(init.signal, controller.signal);
    init.signal.addEventListener("abort", () => reject(new Error("fixture-secret")), { once: true });
    controller.abort();
  }) });
  const send = h.protocol.inference(async () => auth, auth, models);
  await assert.rejects(send(`${BASE}/responses`, { body: JSON.stringify({ model: "muse-spark-1.3", input: [] }), signal: controller.signal }), /inference cancelled/);
  assert.equal(h.requests.length, 1);
});

test("quota and inference error bodies are redacted; Retry-After blocks repeated inference", async () => {
  const h = harness({ inference: () => new Response("fixture-secret", { status: 429, headers: { "retry-after": "60" } }) });
  const send = h.protocol.inference(async () => auth, auth, models);
  const body = JSON.stringify({ model: "muse-spark-1.3", input: [] });
  const response = await send(`${BASE}/responses`, { body });
  assert.equal((await response.text()).includes("fixture-secret"), false);
  await assert.rejects(send(`${BASE}/responses`, { body }), /rate limited/);
  assert.equal(h.requests.length, 1);
  h.advance(60001);
  await send(`${BASE}/responses`, { body });
  assert.equal(h.requests.length, 2);
});
