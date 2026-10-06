import assert from "node:assert/strict";
import test from "node:test";
import plugin from "./server.js";

test("v2 hook injects only the current subscription credential at the Responses boundary", async (t) => {
  const previousFetch = globalThis.fetch;
  globalThis.fetch = async (url) => {
    assert.equal(url, "https://api.meta.ai/v1/models");
    return Response.json({ data: [{ id: "muse-spark-1.3-contributor" }] });
  };
  t.after(() => { globalThis.fetch = previousFetch; });
  let key = "fixture-api-key";
  let requestHook;
  const transforms = [];
  const dispose = await plugin.setup({
    options: {},
    integration: {
      transform: async (register) => register({ update: (_, change) => change({}), method: { update: () => {} } }),
      connection: {
        active: async () => ({ id: "fixture-connection" }),
        resolve: async () => ({ type: "oauth", access: JSON.stringify({ accountToken: "fixture-account-token", apiKey: key }), metadata: { accountID: "fixture-account" } }),
      },
    },
    provider: { transform: async (register) => { transforms.push(register); }, reload: async () => {} },
    session: { hook: async (name, hook, filter) => {
      assert.equal(name, "http.request"); assert.equal(filter.providerID, "muse-code"); requestHook = hook;
    } },
    event: { async *subscribe({ signal }) { await new Promise(resolve => signal.addEventListener("abort", resolve, { once: true })); } },
  });
  t.after(dispose);
  let registered;
  transforms[0]({ add: (value) => { registered = value; } });
  assert.equal(registered.models.length, 1);
  assert.ok(registered.models[0].variants.some(v => v.id === "xhigh"));
  assert.equal(registered.info.settings.apiKey, "muse-subscription-managed");
  const event = { request: new Request("https://api.meta.ai/v1/responses", {
    method: "POST", body: JSON.stringify({ model: "muse-spark-1.3-contributor", reasoning: { effort: "xhigh" }, input: [] }),
  }) };
  await requestHook(event);
  assert.equal(event.request.headers.get("authorization"), "Bearer fixture-api-key");
  assert.equal((await event.request.json()).store, false);
  await assert.rejects(requestHook({ request: new Request("https://example.org/", { method: "POST" }) }), /unverified/);
  key = "different-fixture-key";
  await assert.rejects(requestHook({ request: new Request("https://api.meta.ai/v1/responses", { method: "POST" }) }), /connection changed/);
});
