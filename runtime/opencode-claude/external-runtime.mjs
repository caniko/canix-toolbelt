// External services own their lifecycle; a plugin only releases its hooks.
export async function acquireExternalRuntime(value, ctx) {
  if (typeof value !== "string") throw new Error("externalBaseURL must be a loopback HTTP URL");
  const url = new URL(value);
  if (url.protocol !== "http:" || url.hostname !== "127.0.0.1" || !url.port ||
      url.username || url.password || url.pathname !== "/" || url.search || url.hash) {
    throw new Error("externalBaseURL must be a loopback HTTP origin with an explicit port");
  }
  const registrations = [];
  const release = async () => {
    const results = await Promise.allSettled(registrations.splice(0).map(r => r.dispose()));
    const failures = results.filter(r => r.status === "rejected").map(r => r.reason);
    if (failures.length) throw new AggregateError(failures, "Failed to release Meridian directory hooks");
  };
  try {
    if (ctx) {
      for (const name of ["context", "title", "compaction", "generate"]) {
        registrations.push(await ctx.session.hook(name, async event => {
          const session = await ctx.session.get({ sessionID: event.sessionID });
          const directory = session.location.directory;
          if (typeof directory !== "string" || !directory.startsWith("/") || /[\r\n<>]/.test(directory)) {
            throw new Error("Meridian requires a valid local session directory");
          }
          // Meridian 1.79 extracts cwd from this block. No process-global cwd
          // override: different sessions in a shared backend can use different projects.
          event.system.push({ type: "text", text: `<env>\nWorking directory: ${directory}\n</env>` });
        }, { providerID: "anthropic" }));
      }
    }
  } catch (error) {
    await release();
    throw error;
  }
  return { baseURL: url.origin, release };
}
