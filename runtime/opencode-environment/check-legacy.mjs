// Verify the narrow patch against a real checkout without modifying that checkout.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { copyFile, mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const source = process.argv[2];
if (!source) throw new Error("Usage: node runtime/opencode-environment/check-legacy.mjs /path/to/legacy-opencode");
const root = await mkdtemp(path.join(tmpdir(), "harbor-opencode-contract-"));
try {
  const files = ["packages/opencode/src/tool/shell.ts", "packages/opencode/src/plugin/shell-environment.ts", "packages/plugin/src/index.ts", "packages/core/src/tool/bash.ts"];
  for (const file of files) {
    await mkdir(path.dirname(path.join(root, file)), { recursive: true });
    await copyFile(path.join(source, file), path.join(root, file));
  }
  const original = await readFile(path.join(root, files[0]), "utf8");
  execFileSync("git", ["apply", "--check", fileURLToPath(new URL("./legacy.patch", import.meta.url))], { cwd: root });
  execFileSync("git", ["apply", fileURLToPath(new URL("./legacy.patch", import.meta.url))], { cwd: root });
  const patched = await readFile(path.join(root, files[0]), "utf8");
  const permission = (text) => text.match(/const ask = [\s\S]*?(?=function toolOomCommand)/)?.[0];
  assert.ok(permission(original), "cannot identify the permission boundary");
  assert.equal(permission(patched), permission(original), "permission implementation changed");
  assert.equal(patched, original, "the shell execution and permission implementation must remain unchanged");
  assert.ok(patched.indexOf("yield* ask(ctx, scan, params)") < patched.indexOf("const environment = yield* shellEnv(ctx, cwd)"));
  assert.ok(patched.indexOf("yield* ask(ctx, scan, params)") >= 0);
  const environment = await readFile(path.join(root, files[1]), "utf8");
  assert.ok(environment.includes("OPENCODE_TOOL_OOM_SCORE_ADJ: process.env.OPENCODE_TOOL_OOM_SCORE_ADJ"));
  assert.ok(environment.includes("harborLlm: 1, harborCanixLlm: 1"));
  assert.ok(environment.indexOf("if (extra.harborLlmReplace || extra.harborCanixLlmReplace)") < environment.indexOf("for (const key"));
  const core = await readFile(path.join(root, files[3]), "utf8");
  const guard = core.indexOf('process.env.CANIX_TOOLBELT_OPENCODE_REQUIRE_LEGACY === "1"');
  assert.ok(guard > core.indexOf("yield* permission.assert({", core.indexOf("const source =")));
  assert.ok(guard < core.indexOf("ChildProcess.make(input.command"));
  console.log("OpenCode patch applicability, unchanged shell execution, permission ordering, replacement branch, and OOM contract passed");
} finally {
  await rm(root, { recursive: true, force: true });
}
