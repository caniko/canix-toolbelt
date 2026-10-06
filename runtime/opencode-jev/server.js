// Provider authentication runs first; redirect only declared inference URLs.
export default {
  id: "canix.jev-gateway",
  async setup(ctx) {
    const routes = new Map(Object.entries(ctx.options.routes));
    for (const destination of routes.values()) {
      const url = new URL(destination);
      if (url.protocol !== "http:" || url.hostname !== "127.0.0.1" || url.username || url.password) {
        throw new Error("Jev must use a loopback gateway");
      }
    }
    const registration = await ctx.session.hook("http.request", event => {
      if (event.request.method !== "POST") return;
      const url = new URL(event.request.url);
      const destination = routes.get(`${url.origin}${url.pathname}`);
      if (!destination) {
        if (/(?:\/(?:responses|chat\/completions|messages)|:(?:streamGenerateContent|generateContent))\/?$/.test(url.pathname)) {
          throw new Error(`Jev route missing for ${url.origin}${url.pathname}; configure this inference endpoint before using it`);
        }
        return;
      }
      const target = new URL(destination);
      target.search = url.search;
      event.request = new Request(target, event.request);
    });
    return () => registration.dispose();
  },
};
