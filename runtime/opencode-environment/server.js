import path from "node:path";
import { pathToFileURL } from "node:url";

// Keep the old persistent anchors throughout the mixed-version transition.
// A namespace change requires draining old processes; never unlink an anchor.
export function projectEnvironmentOptions(options, environment = process.env) {
  return {
    ...options,
    preparationLockDirectory: options.preparationLockDirectory
      ?? path.join(environment.XDG_RUNTIME_DIR ?? path.join(environment.HOME, ".cache"), "harbor-canix-llm"),
  };
}

export default {
  id: "canix-toolbelt.project-environment",
  async setup(ctx) {
    const { plugin, ...options } = ctx.options;
    if (typeof plugin !== "string" || !plugin.startsWith("/nix/store/") || path.normalize(plugin) !== plugin) {
      throw new Error("Toolbelt requires an explicit immutable Harbor plugin path");
    }
    const adapter = (await import(pathToFileURL(plugin).href)).default;
    return adapter.setup({ ...ctx, options: projectEnvironmentOptions(options) });
  },
};
