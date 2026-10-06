import assert from "node:assert/strict";
import test from "node:test";
import adapter, { projectEnvironmentOptions } from "./server.js";

test("transition uses the previous runtime and fallback lock locations", () => {
  assert.equal(projectEnvironmentOptions({}, { HOME: "/home/example", XDG_RUNTIME_DIR: "/run/user/1001" }).preparationLockDirectory, "/run/user/1001/harbor-canix-llm");
  assert.equal(projectEnvironmentOptions({}, { HOME: "/home/example" }).preparationLockDirectory, "/home/example/.cache/harbor-canix-llm");
  assert.deepEqual(projectEnvironmentOptions({ preparationLockDirectory: "/shared/locks", roots: ["/project"] }, {}), {
    preparationLockDirectory: "/shared/locks", roots: ["/project"],
  });
});

test("mutable payloads are rejected before loading a generic adapter", async () => {
  for (const plugin of [undefined, "/workspace/plugin.mjs", "/nix/store/../mutable/plugin.mjs"]) {
    await assert.rejects(adapter.setup({options: {plugin}}), /immutable Harbor plugin/);
  }
});
