{lib}: {
  mkReverseProxyRoute = {
    hostname,
    port,
    host ? "localhost",
    injectAnalytics ? false,
    goatcounterUrl ? null,
  }: {
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
      ++ [
        {
          handler = "reverse_proxy";
          upstreams = [{dial = "${host}:${toString port}";}];
        }
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
