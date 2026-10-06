import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { BASE, ID, NAME, createProtocol } from "./protocol.mjs";

// Read-only bridge: the public plugin client has auth.set/remove but no auth.get.
// Writes and file modes are exclusively OpenCode's responsibility (OAuth callback).
async function storedAuth() {
  try {
    const raw = process.env.OPENCODE_AUTH_CONTENT ?? await readFile(join(process.env.XDG_DATA_HOME || join(homedir(), ".local/share"), "opencode/auth.json"), "utf8");
    return JSON.parse(raw)[ID];
  } catch (error) {
    if (error.code === "ENOENT") return undefined;
    throw new Error("Cannot read OpenCode credentials. Check auth.json permissions and JSON syntax.");
  }
}

let sharedProtocol;
export default async function MuseCodePlugin() {
  // One exchange/quota cache across project instances in a shared OpenCode server.
  const protocol = sharedProtocol ??= createProtocol();
  const lifetime = new AbortController();
  let login;
  let discovery = { state: "not-connected" };
  let discoveredAuth;
  let models = {};
  return {
    dispose: async () => { login?.abort(); lifetime.abort(); },
    config: async config => {
      if (config.disabled_providers?.includes(ID)) return;
      if (config.enabled_providers && !config.enabled_providers.includes(ID)) return;
      config.provider ??= {};
      const auth = await storedAuth();
      models = {};
      discoveredAuth = auth;
      if (auth) {
        try {
          const result = await protocol.discover(auth, lifetime.signal);
          models = result.models;
          discovery = { state: "available", entitledCount: result.entitledCount, unsupportedCount: result.unsupportedCount };
        } catch (error) {
          discovery = { state: "unavailable", message: error.message };
        }
      }
      // Exact provider ownership, no config/env API key, static seed or PAYG fallback.
      config.provider[ID] = { name: NAME, npm: "@ai-sdk/openai", api: BASE, env: [], whitelist: Object.keys(models), models };
    },
    auth: {
      provider: ID,
      methods: [{ type: "oauth", label: "Muse Code subscription (device authorization)", authorize: async () => {
        login?.abort();
        login = new AbortController();
        return protocol.authorize(AbortSignal.any([lifetime.signal, login.signal]));
      } }],
      loader: async getAuth => ({
        apiKey: "opencode-oauth-dummy-key", baseURL: BASE,
        fetch: protocol.inference(getAuth, discoveredAuth ?? await getAuth(), models),
      }),
    },
    "chat.params": async (input, output) => {
      if (input.model.providerID !== ID) return;
      output.maxOutputTokens = Math.min(output.maxOutputTokens ?? input.model.limit.output, input.model.limit.output);
      // store=false and encrypted-reasoning include come from the shared
      // Meta/muse-code handling in transform.ts, not duplicated here.
    },
    tool: {
      muse_code_status: {
        description: "Show redacted Muse Code subscription quota and model-discovery status. Cached for five minutes; no API-equivalent dollar charges.",
        args: {},
        execute: async (_args, context) => {
          const auth = await storedAuth();
          if (!auth) return JSON.stringify({ subscription: "not-connected", discovery: { state: "not-connected" } });
          const currentDiscovery = auth.access === discoveredAuth?.access ? discovery : { state: "account-changed", message: "Restart OpenCode to refresh discovery." };
          try {
            return JSON.stringify({ ...await protocol.status(auth, context.abort), discovery: currentDiscovery });
          } catch (error) {
            return JSON.stringify({ subscription: "unavailable", message: error.message, discovery: currentDiscovery });
          }
        },
      },
    },
  };
}
