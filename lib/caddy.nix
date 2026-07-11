{lib}: let
  redirectLocation = {
    to,
    preservePath ? true,
  }:
    if preservePath
    then "${to}{http.request.uri}"
    else to;
in {
  # Build a Caddy route with caddy-security authenticate handler
  # guarding a reverse proxy to the upstream service. The portal
  # must match a key in canix-toolbelt.services.caddy.authProviders.
  mkAuthServiceRoute = {
    hostname,
    port,
    host ? "localhost",
    upstreamScheme ? "http",
    tlsServerName ? null,
    portalName,
    cookieDomain ? null,
  }: let
    proxyHandler =
      {
        handler = "reverse_proxy";
        upstreams = [{dial = "${host}:${toString port}";}];
      }
      // lib.optionalAttrs (upstreamScheme == "https") {
        transport = {
          protocol = "http";
          tls.server_name =
            if tlsServerName != null
            then tlsServerName
            else hostname;
        };
      };
  in {
    match = [{host = [hostname];}];
    handle = [
      {
        handler = "authenticator";
        portal_name = portalName;
        route_matcher = "*";
      }
      {
        handler = "subroute";
        routes = [{handle = [proxyHandler];}];
      }
    ];
  };

  mkReverseProxyRoute = {
    hostname,
    port,
    host ? "localhost",
    upstreamScheme ? "http",
    tlsServerName ? null,
    injectAnalytics ? false,
    goatcounterUrl ? null,
  }: let
    proxyHandler =
      {
        handler = "reverse_proxy";
        upstreams = [{dial = "${host}:${toString port}";}];
      }
      // lib.optionalAttrs (upstreamScheme == "https") {
        transport = {
          protocol = "http";
          tls.server_name =
            if tlsServerName != null
            then tlsServerName
            else hostname;
        };
      };
  in {
    match = [{host = [hostname];}];
    handle =
      (lib.optional injectAnalytics {
        handler = "replace_response";
        match = {
          headers."Content-Type" = ["text/html*"];
        };
        replacements = [
          {
            search = "</head>";
            replace = ''<script data-goatcounter="${goatcounterUrl}/count" async src="${goatcounterUrl}/count.js"></script></head>'';
          }
        ];
      })
      ++ [proxyHandler];
  };

  mkRedirectRoute = {
    from,
    to,
    status ? 301,
    preservePath ? true,
  }: {
    match = [{host = [from];}];
    handle = [
      {
        handler = "static_response";
        status_code = status;
        headers.Location = [
          (redirectLocation {
            inherit to preservePath;
          })
        ];
      }
    ];
  };

  mkStaticResponseRoute = {
    hostname,
    status,
    location ? null,
    path ? null,
    queryNot ? null,
  }: {
    match = [
      ({
        host = [hostname];
      }
      // lib.optionalAttrs (path != null) {
        path =
          if builtins.isList path
          then path
          else [path];
      }
      // lib.optionalAttrs (queryNot != null) {
        not = [{query = queryNot;}];
      })
    ];
    handle = [
      ({
        handler = "static_response";
        status_code = status;
      }
      // lib.optionalAttrs (location != null) {
        headers.Location = [location];
      })
    ];
  };

  mkStaticFileRoute = {
    hostname,
    root,
  }: {
    match = [{host = [hostname];}];
    handle = [
      {
        handler = "vars";
        inherit root;
      }
      {
        handler = "file_server";
      }
    ];
  };
}
