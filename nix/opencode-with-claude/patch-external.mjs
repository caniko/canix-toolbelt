// Compatibility patch for the immutable 1.11.1 npm artifact. Fail on drift.
import { readFileSync, writeFileSync, copyFileSync } from "node:fs";
import { dirname, join } from "node:path";

const [entry, runtime] = process.argv.slice(2);
const source = readFileSync(entry, "utf8");
const before = "{release:o,baseURL:i}=await ce(n),s=";
const after = "{release:o,baseURL:i}=await (e.options?.externalBaseURL===undefined?ce(n):acquireExternalRuntime(e.options.externalBaseURL,e)),s=";
if (source.split(before).length !== 2) throw new Error("opencode-with-claude 1.11.1 external-runtime patch drift");
writeFileSync(entry, 'import { acquireExternalRuntime } from "./external-runtime.mjs";\n' + source.replace(before, after));
copyFileSync(runtime, join(dirname(entry), "external-runtime.mjs"));
