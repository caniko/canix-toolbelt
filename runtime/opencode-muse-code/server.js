// V2 entrypoint. Reuse the subscription protocol; credentials are owned by
// native v2 integrations and injected only at the verified HTTP boundary.
import { BASE, ID, NAME, credential, createProtocol, serialize } from "./protocol.mjs";
import { readFile, stat } from "node:fs/promises";
import path from "node:path";

const nativeCredential = (auth, methodID) => ({
  type: "oauth", methodID, access: auth.access, refresh: auth.refresh ?? "",
  expires: auth.expires, metadata: { accountId: auth.accountId },
});

export function inventory(models) {
  return Object.entries(models).map(([id, model]) => ({
    id, modelID: id, providerID: ID, name: model.name,
    capabilities: { tools: true, input: model.modalities.input, output: model.modalities.output },
    variants: Object.keys(model.variants).map(id => ({ id, settings: { reasoningEffort: id, reasoningSummary: "auto", store: false, include: ["reasoning.encrypted_content"] } })),
    settings: { store: false, reasoningSummary: "auto", include: ["reasoning.encrypted_content"] },
    time: { released: 0 }, cost: [], status: "active", enabled: true, limit: model.limit,
  }));
}

export default {
  id: "canix-toolbelt.provider.muse-code",
  async setup(ctx) {
    const protocol = createProtocol();
    const lifetime = new AbortController();
    let models = {}, expected, sourceConnection;
    const authFor = async () => {
      const connection = await ctx.integration.connection.active(ID);
      if (!connection) return undefined;
      const value = await ctx.integration.connection.resolve(connection);
      if (value?.type !== "oauth") throw new Error("Muse Code requires a subscription OAuth connection");
      return { connection, auth: { ...value, accountId: value.metadata?.accountId ?? value.metadata?.accountID } };
    };
    await ctx.integration.transform(editor => {
      editor.update(ID, integration => { integration.name = NAME; });
      editor.method.update({
        integrationID: ID,
        method: { id: "device", type: "oauth", label: "Muse Code subscription" },
        authorize: async () => {
          const login = await protocol.authorize(lifetime.signal);
          return {
            url: login.url, instructions: login.instructions, mode: "auto",
            callback: login.callback().then(result => nativeCredential(result,"device")),
          };
        },
      });
      if (ctx.options.legacyAuthFile) {
        editor.method.update({
          integrationID: ID,
          method: { id: "import-v1", type: "oauth", label: "Import existing local Muse subscription" },
          authorize: async () => {
            const file = ctx.options.legacyAuthFile;
            if (typeof file !== "string" || !path.isAbsolute(file)) throw new Error("Muse import requires an absolute credential file");
            const info = await stat(file);
            if (!info.isFile() || (info.mode & 0o077) !== 0) throw new Error("Muse credential source must be a private file");
            const auth = JSON.parse(await readFile(file,"utf8"))[ID];
            await protocol.discover(auth,lifetime.signal);
            return { url:"https://auth.meta.com/", instructions:"Importing the existing local subscription into native v2 credential storage. No browser action is needed.", mode:"auto", callback:Promise.resolve(nativeCredential(auth,"import-v1")) };
          },
        });
      }
    });
    await ctx.provider.transform(editor => editor.add({
      info: { id: ID, name: NAME, integrationID: ID, activation: "enabled",
        package: "@opencode/ai/providers/openai",
        settings: { baseURL: BASE, apiKey: "muse-subscription-managed", transport: "http", store: false, compaction: { type: "summary" } },
      },
      models: inventory(models), sourceConnection,
    }));
    const refresh = async () => {
      const current = await authFor();
      if (current) {
        const result = await protocol.discover(current.auth, lifetime.signal);
        models = result.models; expected = credential(current.auth); sourceConnection = current.connection;
      } else { models = {}; expected = undefined; sourceConnection = undefined; }
      await ctx.provider.reload();
    };
    await ctx.session.hook("http.request", async event => {
      if (event.request.url !== `${BASE}/responses` || event.request.method !== "POST") throw new Error("Muse refused an unverified inference destination");
      const current = await authFor();
      const auth = credential(current?.auth);
      if (!expected || auth.apiKey !== expected.apiKey || auth.accountId !== expected.accountId) throw new Error("Muse connection changed; refresh its model catalog before continuing");
      const body = serialize(await event.request.clone().text(), models);
      const headers = new Headers(event.request.headers);
      headers.set("Authorization", `Bearer ${auth.apiKey}`);
      headers.set("x-api-version", "1.0.0");
      event.request = new Request(event.request, { headers, body, redirect: "error" });
    }, { providerID: ID });
    await refresh();
    const watch = (async () => {
      for await (const event of ctx.event.subscribe({ signal: lifetime.signal })) {
        if (event.type === "credential.updated" || event.type === "credential.switched") {
          try { await refresh(); }
          catch { models = {}; expected = undefined; sourceConnection = undefined; await ctx.provider.reload(); }
        }
      }
    })().catch(async () => {
      if (lifetime.signal.aborted) return;
      models = {}; expected = undefined; sourceConnection = undefined;
      await ctx.provider.reload();
      console.error("Muse credential event stream failed; catalog disabled until plugin reload");
    });
    return async () => { lifetime.abort(); await watch.catch(() => {}); };
  },
};
