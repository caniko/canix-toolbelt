{lib}: {
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
