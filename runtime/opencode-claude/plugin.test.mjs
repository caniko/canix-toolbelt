import assert from "node:assert/strict";
import { test } from "node:test";
import { createServer } from "node:http";
import { pathToFileURL } from "node:url";

const { default: plugin } = await import(pathToFileURL(process.env.OPENCODE_CLAUDE_PLUGIN));

function context(baseURL) {
  const hooks = new Map();
  let disposed = 0;
  return {
    hooks,
    invoke: async (name, event) => {
      for (const hook of hooks.get(name) || []) await hook(event);
    },
    disposed: () => disposed,
    ctx: {
      options: { externalBaseURL: baseURL },
      agent: { transform: async fn => {
        fn({ list: () => [{ id: "build", mode: "primary" }, { id: "general", mode: "subagent" }] });
        return { dispose: async () => disposed++ };
      } },
      session: { get: async ({ sessionID }) => ({ location: { directory: `/projects/${sessionID}` } }), hook: async (name, callback, scope) => {
        assert.equal(scope.providerID, "anthropic");
        hooks.set(name, [...(hooks.get(name) || []), callback]);
        return { dispose: async () => disposed++ };
      } },
    },
  };
}

test("packaged V2 plugin uses an occupied external service without binding or closing it", async () => {
  const server = createServer((_, res) => res.end("alive"));
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const first = context(origin);
  const second = context(origin);
  let closeFirst, closeSecond;
  try {
    closeFirst = await plugin.setup(first.ctx);
    closeSecond = await plugin.setup(second.ctx);
    for (const kind of ["primary", "title", "compaction", "generate"]) {
      const event = { sessionID: "session-a", agent: "build", kind, baseURL: "https://api.anthropic.com", headers: {} };
      await first.invoke("model.request", event);
      assert.equal(event.baseURL, `${origin}/v1`);
      if (kind === "primary" || kind === "compaction") assert.equal(event.headers["x-opencode-session"], "session-a");
      else assert.equal(event.headers["x-opencode-session"], undefined);
      if (kind === "compaction") assert.equal(event.headers["x-meridian-source"], "subagent-compaction");
      event.request = new Request(`${event.baseURL}/messages`, { method: "POST", headers: { ...event.headers, "anthropic-beta": "transport-added-beta" }, body: "{}" });
      await first.invoke("http.request", event);
      assert.equal(event.request.headers.get("anthropic-beta"), null);
    }
    const system = [
      { type: "text", text: "User instructions from AGENTS.md: keep this context.\n<env>\nWorking directory: /stale\nPlatform: linux\n</env>" },
      { type: "text", text: "<ENV>working directory: /inline-stale</ENV>" },
    ];
    for (const name of ["context", "title", "compaction", "generate"]) {
      assert.equal(first.hooks.get(name).length, 2);
      const event = { sessionID: "session-a", system: structuredClone(system) };
      for (let replay = 0; replay < 2; replay++) {
        await first.invoke(name, event);
        const text = event.system.map(p => p.text).join("\n");
        assert.match(text, /keep this context/);
        assert.match(text, /Platform: linux/);
        assert.match(text, /<env>\s*Working directory: \/projects\/session-a\s*<\/env>/);
        assert.equal(text.match(/Working directory:/gi).length, 1);
        assert.doesNotMatch(text, /\/stale|\/inline-stale/);
      }
    }
    await closeFirst();
    closeFirst = undefined;
    assert.equal(first.disposed(), 11);
    assert.equal(await (await fetch(origin)).text(), "alive");
    const continuation = { sessionID: "session-b", agent: "general", kind: "primary", headers: {} };
    await second.invoke("model.request", continuation);
    assert.equal(continuation.baseURL, `${origin}/v1`);
    assert.equal(continuation.headers["x-opencode-session"], "session-b");
    assert.equal(continuation.headers["x-opencode-agent-mode"], "subagent");
    const other = { sessionID: "session-b", system: structuredClone(system) };
    await second.invoke("context", other);
    assert.match(other.system.map(p => p.text).join("\n"), /Working directory: \/projects\/session-b/);
  } finally {
    await closeFirst?.();
    await closeSecond?.();
    await new Promise(resolve => server.close(resolve));
  }
});
