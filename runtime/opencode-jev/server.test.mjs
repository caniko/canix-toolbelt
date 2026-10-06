import assert from "node:assert/strict";
import { test } from "node:test";
import plugin from "./server.js";

test("inference redirects preserve headers, streaming bodies and cancellation", async () => {
  let hook, disposed = false;
  const cleanup = await plugin.setup({
    options: { routes: { "http://127.0.0.1:3460/v1/messages": "http://127.0.0.1:8792/v1/messages" } },
    session: { hook: async (_, callback) => {
      hook = callback;
      return { dispose: async () => { disposed = true; } };
    } },
  });
  const controller = new AbortController();
  const payload = JSON.stringify({ model: "selected-model", messages: [] });
  const event = { request: new Request("http://127.0.0.1:3460/v1/messages?beta=true", {
    method: "POST", headers: { "x-api-key": "user-key", "x-opencode-session": "session-a" },
    body: new ReadableStream({ start(c) { c.enqueue(new TextEncoder().encode(payload)); c.close(); } }),
    duplex: "half", signal: controller.signal,
  }) };
  hook(event);
  assert.equal(event.request.url, "http://127.0.0.1:8792/v1/messages?beta=true");
  assert.equal(event.request.headers.get("x-api-key"), "user-key");
  assert.equal(event.request.headers.get("x-opencode-session"), "session-a");
  assert.equal(await event.request.text(), payload);
  controller.abort();
  assert.equal(event.request.signal.aborted, true);
  for (const path of ["/v1/messages", "/v1/responses", "/v1/chat/completions", "/v1beta/models/test:streamGenerateContent"]) {
    assert.throws(() => hook({ request: new Request(`https://unconfigured.example${path}`, { method: "POST" }) }), /Jev route missing/);
  }
  for (const [method, path] of [["GET", "/v1/models"], ["POST", "/oauth/token"]]) {
    const request = new Request(`https://provider.example${path}`, { method });
    const other = { request }; hook(other); assert.equal(other.request, request);
  }
  await cleanup();
  assert.equal(disposed, true);
});

test("gateway destinations require loopback HTTP", async () => {
  for (const url of ["https://127.0.0.1:8792/v1/messages", "http://example.com:8792/v1/messages", "http://user:key@127.0.0.1:8792/v1/messages"]) {
    await assert.rejects(plugin.setup({ options: { routes: { source: url } } }), /loopback gateway/);
  }
});
