// Cross-project contract against actual Meridian extraction/resolution. Supply
// CLAUDE_PLUGIN_FILE (built or installed JS) and MERIDIAN_SOURCE (source checkout).
import assert from "node:assert/strict";
import { mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { test } from "node:test";

if (!process.env.CLAUDE_PLUGIN_FILE || !process.env.MERIDIAN_SOURCE) {
  throw new Error("CLAUDE_PLUGIN_FILE and MERIDIAN_SOURCE are required; use qualified installed artifacts");
}
const { default: plugin } = await import(pathToFileURL(resolve(process.env.CLAUDE_PLUGIN_FILE)));
const { extractClientCwd } = await import(
  pathToFileURL(resolve(process.env.MERIDIAN_SOURCE, "src/proxy/session/fingerprint.ts"))
);
const { resolveSdkWorkingDirectory } = await import(
  pathToFileURL(resolve(process.env.MERIDIAN_SOURCE, "src/proxy/cwd.ts"))
);

test("canonical session cwd survives scrubbing, replays and actual Meridian extraction/resolution", async () => {
  const root = await mkdtemp(`${tmpdir()}/meridian-cwd-`);
  const local = `${root}/project`;
  const override = `${root}/override`;
  await Promise.all([mkdir(local), mkdir(override)]);
  const directories = {
    local,
    remote: `/unavailable-fixture-${root.split("/").at(-1)}`,
    windows: "C:\\projects\\fixture",
    unc: "\\\\server\\share\\fixture",
  };
  const hooks = new Map();
  const envBefore = [process.env.MERIDIAN_WORKDIR, process.env.CLAUDE_PROXY_WORKDIR];
  const cleanup = await plugin.setup({
    options: { externalBaseURL: "http://127.0.0.1:3460" },
    agent: { transform: async () => ({ dispose: async () => {} }) },
    session: {
      get: async ({ sessionID }) => ({ location: { directory: directories[sessionID] } }),
      hook: async (name, callback) => {
        hooks.set(name, callback);
        return { dispose: async () => {} };
      },
    },
  });
  try {
    for (const hook of ["context", "title", "compaction", "generate"]) {
      await Promise.all(
        Object.entries(directories).map(async ([sessionID, directory]) => {
          const input = {
            sessionID,
            model: { providerID: "anthropic" },
            system: [
              { type: "text", text: "Preserve user instructions.\n<env>\nWorking directory: /stale\n</env>" },
              { type: "text", text: "<env>\nWorking directory: /also-stale\n</env>" },
              { type: "text", text: "<ENV>\nworking directory: /case-stale\n</ENV>" },
              { type: "text", text: "<env>Working directory: /inline-stale</env>" },
            ],
          };
          for (let replay = 0; replay < 2; replay++) {
            await hooks.get(hook)(input);
            assert.equal(extractClientCwd(input), directory, `${hook}/${sessionID} extraction`);
            assert.equal(
              input.system
                .map((part) => part.text)
                .join("\n")
                .match(/Working directory:/gi)?.length,
              1,
            );
            assert.match(input.system.map((part) => part.text).join("\n"), /Preserve user instructions/);
          }
          if (sessionID === "local" || sessionID === "remote") {
            const resolved = resolveSdkWorkingDirectory({ adapterCwd: extractClientCwd(input), fallback: root });
            assert.equal(resolved.workingDirectory, sessionID === "local" ? local : root);
            assert.equal(resolved.claimedWorkingDirectory, directory);
            assert.equal(resolved.fellBack, sessionID === "remote");
            assert.equal(
              resolveSdkWorkingDirectory({ envOverride: override, adapterCwd: extractClientCwd(input), fallback: root })
                .workingDirectory,
              override,
            );
          }
        }),
      );
    }
    assert.deepEqual([process.env.MERIDIAN_WORKDIR, process.env.CLAUDE_PROXY_WORKDIR], envBefore);
  } finally {
    await cleanup();
    await rm(root, { recursive: true, force: true });
  }
});
