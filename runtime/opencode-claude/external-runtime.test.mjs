import assert from "node:assert/strict";
import { test } from "node:test";
import { acquireExternalRuntime } from "./external-runtime.mjs";

test("external instances retain their endpoint and leave service ownership external", async () => {
  const listeners = process.listenerCount("SIGTERM");
  const first = await acquireExternalRuntime("http://127.0.0.1:4567/");
  const second = await acquireExternalRuntime("http://127.0.0.1:4568");
  assert.equal(first.baseURL, "http://127.0.0.1:4567");
  assert.equal(second.baseURL, "http://127.0.0.1:4568");
  await first.release();
  await first.release();
  assert.equal(second.baseURL, "http://127.0.0.1:4568");
  assert.equal(process.listenerCount("SIGTERM"), listeners);
});

test("external origins reject remote services, credentials and URL suffixes", async () => {
  for (const url of [null, 4567, "https://127.0.0.1:4567", "http://example.com:4567",
    "http://0.0.0.0:4567", "http://127.0.0.1", "http://127.0.0.1:4567/v1",
    "http://user:pass@127.0.0.1:4567", "http://127.0.0.1:4567?key=secret",
    "http://127.0.0.1:4567#fragment"]) {
    await assert.rejects(acquireExternalRuntime(url));
  }
});
