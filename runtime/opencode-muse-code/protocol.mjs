// Protocol adapted from oh-my-pi PR #10677, commit 6785d70d53e2bc10aad259a532f6b5255275991e.
// See LICENSE and README.md. No Meta PAYG credential or endpoint fallback is permitted.
import { setTimeout as sleep } from "node:timers/promises";

export const ID = "muse-code";
export const NAME = "Muse Code (Subscription)";
export const BASE = "https://api.meta.ai/v1";
const DEVICE = "https://auth.meta.com/oidc/device/authorization/";
const TOKEN = "https://auth.meta.com/oidc/device/token/";
const KEY = "https://api.meta.ai/muse-code/key";
const CLIENT = "1031625952748946";
const HEADERS = { Accept: "application/json", "x-api-version": "1.0.0" };
const EFFORTS = ["minimal", "low", "medium", "high", "xhigh"];
// Short display labels; API IDs are unchanged. Contributor models keep their
// suffix: their prompts may be used for training.
const NAMES = {
  "muse-spark-1.1": "Spark 1.1",
  "muse-spark-1.2": "Spark 1.2",
  "muse-spark-1.2-contributor": "Spark 1.2 Contributor",
  "muse-spark-1.3": "Spark 1.3",
  "muse-spark-1.3-contributor": "Spark 1.3 Contributor",
};
const KNOWN = Object.keys(NAMES);

// Variant key order, weakest to strongest, with `max` last only on
// muse-spark-1.3. Variant *settings* (reasoningSummary, encrypted-reasoning
// include) are owned by the shared Meta/muse-code handling in transform.ts,
// which generates the same key order — so the provider merge preserves it
// instead of prepending generic low/medium/high first. The plugin supplies
// keys only: order for the merge, names for effort validation in serialize().
function museVariants(id) {
  const efforts = id === "muse-spark-1.3" ? [...EFFORTS, "max"] : EFFORTS;
  return Object.fromEntries(efforts.map(effort => [effort, {}]));
}

export function object(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Muse Code returned malformed data.");
  return value;
}

function text(value) {
  if (typeof value !== "string" || !value.trim() || /[\r\n\x00]/.test(value)) throw new Error("Muse Code returned a missing or invalid field.");
  return value;
}

export function credential(auth) {
  if (auth?.type !== "oauth") throw new Error("Connect Muse Code (Subscription) with /connect; API keys are not accepted here.");
  try {
    const data = object(JSON.parse(auth.access));
    return { accountToken: text(data.accountToken), apiKey: text(data.apiKey), accountId: text(auth.accountId) };
  } catch {
    throw new Error("Muse Code credentials are invalid. Reconnect with /connect.");
  }
}

function httpError(status) {
  if (status === 401) return new Error("Muse Code authorization expired or was revoked. Reconnect with /connect.");
  if (status === 402) return new Error("Muse Code requires a subscription payment action. Check billing in your Meta account.");
  if (status === 403) return new Error("Muse Code subscription access denied. Check that your subscription is active, then reconnect.");
  return new Error(`Muse Code request failed (HTTP ${status}). No alternate provider was used.`);
}

export function modelsFromResponse(payload) {
  const data = object(payload).data;
  if (!Array.isArray(data)) throw new Error("Muse Code model discovery returned malformed data.");
  const models = Object.create(null);
  const unsupported = [];
  for (const row of data) {
    const id = text(object(row).id);
    // Metadata is verified for these exact revisions only, never extrapolated to future models.
    if (!KNOWN.includes(id)) { unsupported.push(id); continue; }
    for (const field of ["context_length", "max_completion_tokens"]) {
      if (row[field] !== undefined && (!Number.isSafeInteger(row[field]) || row[field] <= 0))
        throw new Error("Muse Code model discovery returned invalid token limits.");
    }
    models[id] = {
      name: NAMES[id],
      reasoning: true,
      tool_call: true,
      attachment: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: Math.min(row.context_length ?? 1048576, 1048576), output: Math.min(row.max_completion_tokens ?? 131072, 131072, row.context_length ?? 1048576) },
      // Zero is unmetered subscription accounting here, not a claim that the plan is free.
      cost: { input: 0, output: 0, cache_read: 0, cache_write: 0 },
      options: { store: false, include: ["reasoning.encrypted_content"] },
      variants: museVariants(id),
    };
  }
  return { models, unsupportedCount: unsupported.length, entitledCount: data.length };
}

export function serialize(body, models) {
  let request;
  try { request = object(JSON.parse(body)); }
  catch { throw new Error("Muse Code requires a valid JSON Responses request."); }
  const model = Object.hasOwn(models, request.model) ? models[request.model] : undefined;
  if (!model) throw new Error("Muse Code model is not in this account's verified discovery. Reconnect or restart OpenCode.");
  if (request.tools !== undefined && (!Array.isArray(request.tools) || request.tools.some(tool =>
    !tool || tool.type !== "function" || typeof tool.name !== "string" || !tool.parameters || typeof tool.parameters !== "object"
  ))) throw new Error("Muse Code supports function tools only; custom/freeform and hosted tools are not supported.");
  if (request.reasoning?.effort !== undefined && !Object.hasOwn(model.variants, request.reasoning.effort))
    throw new Error("Muse Code reasoning effort is not supported by the selected model.");
  if (request.max_output_tokens !== undefined && (!Number.isSafeInteger(request.max_output_tokens) || request.max_output_tokens <= 0))
    throw new Error("Muse Code output token limit must be a positive integer.");
  request.max_output_tokens = Math.min(request.max_output_tokens ?? model.limit.output, model.limit.output);
  request.store = false;
  if (request.include !== undefined && (!Array.isArray(request.include) || request.include.some(value => typeof value !== "string")))
    throw new Error("Muse Code include must be an array of strings.");
  request.include = [...new Set([...(request.include ?? []), "reasoning.encrypted_content"])];
  // Keep input reasoning items, encrypted_content, call IDs and function_call_output untouched.
  return JSON.stringify(request);
}

export function createProtocol({ fetch: send = globalThis.fetch, now = Date.now, wait = (ms, signal) => sleep(ms, undefined, { signal }) } = {}) {
  const backoff = new Map();
  const exchanges = new Map();
  const usage = new Map();

  function retryAfter(response) {
    const value = response.headers.get("retry-after");
    const seconds = Number(value);
    return value && Number.isFinite(seconds) ? Math.max(1000, seconds * 1000)
      : Math.max(1000, Date.parse(value ?? "") - now() || 60000);
  }

  async function request(url, init, scope = url) {
    if (! [DEVICE, TOKEN, KEY, `${BASE}/models`, `${BASE}/responses`].includes(url))
      throw new Error("Muse Code refused an unverified destination.");
    if ((backoff.get(scope) ?? 0) > now()) throw new Error("Muse Code is rate limited. Wait before retrying; no alternate account was used.");
    let response;
    try {
      response = await send(url, {
        ...init, redirect: "error",
        signal: AbortSignal.any([...(init.signal ? [init.signal] : []), AbortSignal.timeout(20000)]),
      });
    } catch {
      if (init.signal?.aborted) throw new Error("Muse Code request cancelled.");
      // Never propagate transport exception messages, bodies, URLs or causes carrying credentials.
      throw new Error("Muse Code network request failed or timed out. Retry when connectivity is restored.");
    }
    if (response.status === 429) {
      backoff.set(scope, now() + retryAfter(response));
      await response.body?.cancel();
      throw new Error("Muse Code is rate limited. Wait before retrying; no alternate account was used.");
    }
    return response;
  }

  async function json(response) {
    if (!response.ok) { await response.body?.cancel(); throw httpError(response.status); }
    try { return object(await response.json()); }
    catch { throw new Error("Muse Code returned malformed JSON."); }
  }

  async function key(accountToken, onboard, signal) {
    // Deduplicate onboarding and quota lookups separately: only the onboard
    // request returns an inference key and identity. Rate limiting stays
    // account-scoped via the shared request scope below.
    const dedup = `key:${onboard ? "onboard" : "quota"}:${accountToken}`;
    if (exchanges.has(dedup)) return exchanges.get(dedup);
    const promise = (async () => {
      const payload = await json(await request(KEY, {
        method: "POST", headers: { ...HEADERS, "Content-Type": "application/json", Authorization: `Bearer ${accountToken}` },
        body: JSON.stringify(onboard ? { onboard: true } : {}), signal,
      }, `key:${accountToken}`));
      if (payload.is_subs_active !== undefined && typeof payload.is_subs_active !== "boolean") throw new Error("Muse Code returned malformed subscription status.");
      if (payload.is_subs_active === false) throw new Error("Muse Code subscription is inactive. Activate it in your Meta account, then reconnect.");
      if (payload.require_payment === true || payload.action_url || payload.require_payment_action_url)
        throw new Error("Muse Code requires a subscription or billing action. Open your Meta account's subscription settings, then reconnect.");
      if (onboard) { text(payload.api_key); text(payload.user_id || payload.user_email); }
      usage.set(accountToken, { at: now(), value: quota(payload) });
      return payload;
    })();
    exchanges.set(dedup, promise);
    try { return await promise; }
    finally { exchanges.delete(dedup); }
  }

  function quota(payload) {
    const result = { subscription: payload.is_subs_active === true ? "active" : "unknown", accounting: "subscription quota, not API charges", windows: [] };
    for (const name of ["window", "weekly"]) {
      const value = payload.subs_usage?.[name];
      if (!value || !Number.isFinite(value.used_percent) || value.used_percent < 0) continue;
      const reset = typeof value.resets_at === "number"
        ? value.resets_at * (value.resets_at < 1000000000000 ? 1000 : 1)
        : typeof value.resets_at === "string" ? Date.parse(value.resets_at) : NaN;
      result.windows.push({ window: name, usedPercent: value.used_percent,
        ...(Number.isFinite(value.window_duration_mins) && value.window_duration_mins > 0 ? { durationMinutes: value.window_duration_mins } : {}),
        ...(reset > 0 && Number.isFinite(new Date(reset).getTime()) ? { resetsAt: new Date(reset).toISOString() } : {}),
      });
    }
    return result;
  }

  return {
    async authorize(signal) {
      const device = await json(await request(DEVICE, { method: "POST", headers: { ...HEADERS, "Content-Type": "application/x-www-form-urlencoded" }, body: new URLSearchParams({ client_id: CLIENT }).toString(), signal }));
      text(device.device_code); text(device.user_code);
      let url;
      try { url = new URL(text(device.verification_uri_complete ?? device.verification_uri)); }
      catch { throw new Error("Muse Code returned an invalid authorization URL."); }
      if (url.origin !== "https://auth.meta.com" || url.username || url.password) throw new Error("Muse Code returned an unverified authorization URL.");
      if (!Number.isFinite(device.expires_in) || device.expires_in <= 0 || device.expires_in > 86400) throw new Error("Muse Code returned an invalid device expiry.");
      if (device.interval !== undefined && (!Number.isFinite(device.interval) || device.interval <= 0)) throw new Error("Muse Code returned an invalid polling interval.");
      const deadline = now() + device.expires_in * 1000;
      let interval = Math.max(1000, Math.floor((device.interval ?? 5) * 1000));
      return {
        url: url.toString(),
        instructions: `Enter code: ${device.user_code}. Approve in your browser; OpenCode completes automatically. Restart OpenCode after login to refresh account models.`,
        // OAuth device authorization: OpenCode invokes this callback once and
        // waits while Meta confirms the browser approval. Nothing is pasted
        // back into OpenCode.
        method: "auto",
        callback: async () => {
          if (signal?.aborted) throw new Error("Muse Code login cancelled.");
          while (now() < deadline) {
            if (signal?.aborted) throw new Error("Muse Code login cancelled.");
            await wait(Math.min(interval, deadline - now()), signal).catch(() => { throw new Error("Muse Code login cancelled."); });
            if (now() >= deadline) break;
            const response = await request(TOKEN, { method: "POST", headers: { ...HEADERS, "Content-Type": "application/x-www-form-urlencoded" }, body: new URLSearchParams({ client_id: CLIENT, device_code: device.device_code, grant_type: "urn:ietf:params:oauth:grant-type:device_code" }).toString(), signal });
            let payload;
            try { payload = object(await response.json()); } catch { throw new Error("Muse Code returned malformed token data."); }
            if (payload.error === "authorization_pending") continue;
            if (payload.error === "slow_down") { interval += 5000; continue; }
            if (payload.error === "access_denied") throw new Error("Muse Code device authorization denied. Start a new login if this was unintended.");
            if (payload.error === "expired_token") break;
            if (!response.ok || payload.error) throw httpError(response.status);
            const accountToken = text(payload.access_token);
            const subscription = await key(accountToken, true, signal);
            text(subscription.api_key); text(subscription.user_id || subscription.user_email);
            if (signal?.aborted) throw new Error("Muse Code login cancelled.");
            return { type: "success", provider: ID, refresh: "", expires: 8640000000000000,
              access: JSON.stringify({ accountToken, apiKey: subscription.api_key }), accountId: subscription.user_id || subscription.user_email };
          }
          throw new Error("Muse Code device code expired. Start a new login with /connect.");
        },
      };
    },
    async discover(auth, signal) {
      const saved = credential(auth);
      return modelsFromResponse(await json(await request(`${BASE}/models`, {
        headers: { ...HEADERS, Authorization: `Bearer ${saved.apiKey}` }, signal,
      }, `models:${saved.apiKey}`)));
    },
    async status(auth, signal) {
      const saved = credential(auth);
      const cached = usage.get(saved.accountToken);
      if (cached && now() - cached.at < 300000) {
        if (cached.error) throw new Error(cached.error);
        return { ...cached.value, cached: true, fetchedAt: cached.at };
      }
      try {
        const value = quota(await key(saved.accountToken, false, signal));
        return { ...value, cached: false, fetchedAt: now() };
      } catch (error) {
        usage.set(saved.accountToken, { at: now(), error: error.message });
        throw error;
      }
    },
    inference(getAuth, expectedAuth, models) {
      return async (input, init = {}) => {
        const url = input instanceof Request ? input.url : String(input);
        if (url !== `${BASE}/responses`) throw new Error("Muse Code refused an unverified inference destination.");
        const expected = credential(expectedAuth);
        const current = credential(await getAuth());
        if (current.accountId !== expected.accountId || current.apiKey !== expected.apiKey)
          throw new Error("Muse Code account changed. Restart OpenCode and select an entitled model again.");
        const body = init.body ?? (input instanceof Request ? await input.text() : undefined);
        if (typeof body !== "string") throw new Error("Muse Code requires a JSON Responses request.");
        const signal = init.signal ?? (input instanceof Request ? input.signal : undefined);
        const serialized = serialize(body, models);
        const scope = `inference:${current.apiKey}`;
        if ((backoff.get(scope) ?? 0) > now()) throw new Error("Muse Code is rate limited. Wait before retrying.");
        let response;
        try {
          response = await send(`${BASE}/responses`, { method: "POST", redirect: "error", signal,
            headers: { ...HEADERS, "Content-Type": "application/json", Authorization: `Bearer ${current.apiKey}` },
            body: serialized });
        } catch {
          if (signal?.aborted) throw new Error("Muse Code inference cancelled.");
          throw new Error("Muse Code inference network request failed. No alternate provider was used.");
        }
        if (response.status === 429) backoff.set(scope, now() + retryAfter(response));
        if (!response.ok) {
          await response.body?.cancel();
          // Do not expose upstream error bodies, which may echo headers or requests.
          return Response.json({ error: { message: response.status === 429 ? "Muse Code quota/rate limit reached; wait before retrying." : httpError(response.status).message } },
            { status: response.status, headers: response.status === 429 ? { "retry-after": String(Math.ceil(retryAfter(response) / 1000)) } : {} });
        }
        return response;
      };
    },
  };
}
