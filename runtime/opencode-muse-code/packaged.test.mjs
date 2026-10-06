// Real compiled OpenCode, disposable HOME, fake remote protocol. Never uses operator credentials.
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, readFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { once } from "node:events";

test("packaged runtime: visibility, OAuth persistence, streaming tools and reasoning, discovery, logout", { skip: !process.env.MUSE_TEST_OPENCODE, timeout: 120000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), "muse-packaged-"));
  let child;
  let logs = "";
  let origin;
  const environment = {
    PATH: process.env.PATH,
    HOME: dir, XDG_CONFIG_HOME: join(dir, "config"), XDG_DATA_HOME: join(dir, "data"),
    XDG_CACHE_HOME: join(dir, "cache"), XDG_STATE_HOME: join(dir, "state"),
    OPENCODE_TEST_HOME: dir, OPENCODE_MODELS_PATH: join(dir, "models.json"), OPENCODE_DISABLE_MODELS_FETCH: "1", OPENCODE_PRINT_LOGS: "1",
    npm_config_offline: "true", npm_config_cache: join(dir, "npm-cache"),
    OPENCODE_CONFIG_CONTENT: JSON.stringify({ plugin: [join(dir, "fixture.mjs")], model: "other/test-model", autoupdate: false, mcp: {}, lsp: false, permission: { read: "allow" }, agent: { title: { disable: true } } }),
  };
  async function start() {
    child = spawn(process.env.MUSE_TEST_OPENCODE, ["serve", "--hostname", "127.0.0.1", "--port", "0"], { cwd: dir, env: environment, stdio: ["ignore", "pipe", "pipe"] });
    let startup = "";
    const capture = data => { logs += data; startup += data; };
    child.stdout.on("data", capture);
    child.stderr.on("data", capture);
    for (let i = 0; i < 200; i++) {
      const match = startup.match(/http:\/\/127\.0\.0\.1:\d+/);
      if (match) { origin = match[0]; return; }
      if (child.exitCode !== null) throw new Error(`Packaged OpenCode exited: ${startup}`);
      await sleep(100);
    }
    throw new Error(`Packaged OpenCode startup timeout: ${startup}`);
  }
  async function stop() {
    if (!child || child.exitCode !== null) return;
    const exit = once(child, "exit");
    child.kill("SIGTERM");
    const ended = await Promise.race([exit.then(() => true), sleep(3000).then(() => false)]);
    if (!ended) { child.kill("SIGKILL"); await exit; }
  }
  async function api(path, body, method = "GET") {
    const response = await fetch(`${origin}${path}`, { method, headers: { "Content-Type": "application/json", "x-opencode-directory": dir }, ...(body ? { body: JSON.stringify(body) } : {}), signal: AbortSignal.timeout(30000) });
    const text = await response.text();
    if (!response.ok) {
      const serverLog = await readFile(join(dir, "data/opencode/log/opencode.log"), "utf8").catch(() => "");
      const diagnostics = `${logs}\n${serverLog}`.replaceAll(/fixture-(?:sub-key|acct-token|payg-key)/g, "[redacted]").slice(-8000);
      assert.fail(`${method} ${path}: ${response.status} ${text.slice(0, 500)}\n${diagnostics}`);
    }
    return JSON.parse(text);
  }
  try {
    await mkdir(join(dir, "config"));
    await writeFile(join(dir, "models.json"), "{}");
    await writeFile(join(dir, "marker.txt"), "muse-tool-roundtrip-marker");
    await writeFile(join(dir, "fixture.mjs"), `
import plugin from ${JSON.stringify(new URL("./index.mjs", import.meta.url).href)};
import { appendFile } from "node:fs/promises";
const original = globalThis.fetch;
globalThis.fetch = async (input, init) => {
  const url = input instanceof Request ? input.url : String(input);
  if (url === "https://auth.meta.com/oidc/device/authorization/") return Response.json({device_code:"fixture-device",user_code:"TEST-CODE",verification_uri:"https://auth.meta.com/device",expires_in:60,interval:0.001});
  if (url === "https://auth.meta.com/oidc/device/token/") return Response.json({access_token:"fixture-acct-token"});
  if (url === "https://api.meta.ai/muse-code/key") return Response.json({api_key:"fixture-sub-key",user_id:"fixture-account",is_subs_active:true});
  if (url === "https://api.meta.ai/v1/models") {
    if(new Headers(init.headers).get("Authorization") !== "Bearer fixture-sub-key") throw new Error("wrong discovery credential");
    return Response.json({data:[{id:"muse-spark-1.3"}]});
  }
  if (url === "https://api.meta.ai/v1/responses") {
    if(new Headers(init.headers).get("Authorization") !== "Bearer fixture-sub-key") throw new Error("wrong inference credential");
    const request = JSON.parse(init.body);
    await appendFile(${JSON.stringify(join(dir, "requests.jsonl"))}, JSON.stringify(request) + "\\n");
    const done = request.input.some(item => item.type === "function_call_output");
    const call = {type:"function_call",status:"completed",id:"fc_test",call_id:"call_test",name:"read",arguments:JSON.stringify({filePath:${JSON.stringify(join(dir, "marker.txt"))}})};
    const reasoning = {type:"reasoning",id:"rs_test",encrypted_content:"dGVzdA==",summary:[]};
    const message = {type:"message",id:"msg_test",role:"assistant",content:[{type:"output_text",text:"muse-stream-ok",annotations:[]}]};
    const events = [{type:"response.created",response:{id:"resp_test",model:"muse-spark-1.3",created_at:1700000000}}];
    if(!done) {
      events.push({type:"response.output_item.added",output_index:0,item:reasoning});
      events.push({type:"response.output_item.done",output_index:0,item:reasoning});
      events.push({type:"response.output_item.added",output_index:1,item:{...call,arguments:""}});
      events.push({type:"response.function_call_arguments.delta",item_id:"fc_test",output_index:1,delta:call.arguments});
      events.push({type:"response.function_call_arguments.done",item_id:"fc_test",output_index:1,arguments:call.arguments});
      events.push({type:"response.output_item.done",output_index:1,item:call});
    } else {
      events.push({type:"response.output_item.added",output_index:0,item:{...message,content:[]}});
      events.push({type:"response.output_text.delta",item_id:"msg_test",output_index:0,content_index:0,delta:"muse-stream-ok"});
      events.push({type:"response.output_item.done",output_index:0,item:message});
    }
    events.push({type:"response.completed",response:{id:"resp_test",status:"completed",output:done?[message]:[reasoning,call],usage:{input_tokens:10,output_tokens:10,input_tokens_details:{cached_tokens:0},output_tokens_details:{reasoning_tokens:1}}}});
    return new Response(events.map(event => "data: " + JSON.stringify(event) + "\\n\\n").join(""),{headers:{"Content-Type":"text/event-stream"}});
  }
  if (url.startsWith("http://127.0.0.1:") || url.startsWith("http://localhost:")) return original(input, init);
  throw new Error("Test blocked external network");
};
export default plugin;
`);
    await start();
    const providers = await api("/provider");
    assert.ok(providers.all, JSON.stringify(providers));
    assert.equal(providers.all.some(p => p.id === "muse-code" && p.name === "Muse Code (Subscription)"), true, JSON.stringify(providers));
    const methods = await api("/provider/auth");
    assert.equal(methods["muse-code"][0].type, "oauth");
    await api("/auth/meta", { type: "api", key: "fixture-payg-key" }, "PUT");
    const authorization = await api("/provider/muse-code/oauth/authorize", { method: 0 }, "POST");
    assert.equal(authorization.method, "auto");
    await api("/provider/muse-code/oauth/callback", { method: 0 }, "POST");
    const authPath = join(dir, "data/opencode/auth.json");
    const saved = JSON.parse(await readFile(authPath, "utf8"));
    assert.equal(saved.meta.key, "fixture-payg-key");
    assert.equal(JSON.parse(saved["muse-code"].access).apiKey, "fixture-sub-key");
    assert.equal((await stat(authPath)).mode & 0o777, 0o600);
    await stop();
    await start();
    const connected = await api("/provider");
    assert.equal(connected.connected.includes("muse-code"), true);
    const muse = connected.all.find(p => p.id === "muse-code");
    assert.deepEqual(Object.keys(muse.models), ["muse-spark-1.3"]);
    assert.equal(muse.models["muse-spark-1.3"].limit.output, 131072);
    assert.equal(muse.models["muse-spark-1.3"].name, "Spark 1.3");
    assert.deepEqual(Object.keys(muse.models["muse-spark-1.3"].variants),
      ["minimal", "low", "medium", "high", "xhigh", "max"]);
    assert.equal(JSON.stringify(connected).includes("fixture-sub-key"), false);
    assert.equal(JSON.stringify(connected).includes("fixture-acct-token"), false);
    const session = await api("/session", {}, "POST");
    const reply = await api(`/session/${session.id}/message`, {
      model: { providerID: "muse-code", modelID: "muse-spark-1.3" }, variant: "max",
      parts: [{ type: "text", text: "Read marker.txt using the read tool and report the result." }],
    }, "POST");
    assert.equal(reply.info.error, undefined, JSON.stringify(reply.info.error));
    assert.ok(reply.parts.some(p => p.type === "text" && p.text.includes("muse-stream-ok")), JSON.stringify(reply));
    const requests = (await readFile(join(dir, "requests.jsonl"), "utf8")).trim().split("\n").map(line => JSON.parse(line));
    assert.ok(requests.length >= 2);
    const toolResult = requests.find(r => r.input.some(item => item.type === "function_call_output"));
    assert.ok(toolResult, JSON.stringify(requests));
    assert.ok(toolResult.input.some(item => item.type === "function_call_output" && JSON.stringify(item.output).includes("muse-tool-roundtrip-marker")));
    assert.ok(toolResult.input.some(item => item.type === "reasoning" && item.encrypted_content === "dGVzdA=="));
    assert.ok(requests.every(r => r.reasoning?.effort === "max"));
    assert.ok(requests.every(r => r.max_output_tokens <= 131072 && r.store === false));
    await api("/auth/muse-code", undefined, "DELETE");
    const loggedOut = JSON.parse(await readFile(authPath, "utf8"));
    assert.equal(loggedOut["muse-code"], undefined);
    assert.equal(loggedOut.meta.key, "fixture-payg-key");
    await stop();
    await start();
    const after = await api("/provider");
    assert.equal(after.connected.includes("muse-code"), false);
    // Exercise the supported CLI automatic completion as well as HTTP auth.
    // The device flow needs no stdin: approval is detected by polling.
    const cli = spawn(process.env.MUSE_TEST_OPENCODE, ["auth", "login", "--provider", "muse-code"], { cwd: dir, env: environment, stdio: ["ignore", "pipe", "pipe"] });
    let transcript = "";
    const capture = data => { transcript += data; };
    cli.stdout.on("data", capture);
    cli.stderr.on("data", capture);
    const cliExit = once(cli, "exit");
    const timeout = setTimeout(() => cli.kill("SIGKILL"), 15000);
    try {
      const [code] = await cliExit;
      assert.equal(code, 0, transcript);
      assert.ok(transcript.includes("Login successful"), transcript);
      const cliSaved = JSON.parse(await readFile(authPath, "utf8"));
      assert.equal(JSON.parse(cliSaved["muse-code"].access).apiKey, "fixture-sub-key");
      assert.equal(cliSaved.meta.key, "fixture-payg-key");
      assert.equal(transcript.includes("fixture-sub-key"), false);
      assert.equal(transcript.includes("fixture-acct-token"), false);
    } finally { clearTimeout(timeout); }
    assert.equal(logs.includes("fixture-sub-key"), false);
    assert.equal(logs.includes("fixture-acct-token"), false);
  } finally {
    await stop();
    await rm(dir, { recursive: true, force: true });
  }
});
