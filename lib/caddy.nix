{lib}: let
  pathValue = path:
    if path.type == "prefix"
    then "${path.value}*"
    else path.value;

  requestMatch = hostname: match:
    {
      host = [hostname];
    }
    // lib.optionalAttrs (match.paths != []) {
      path = map pathValue match.paths;
    }
    // lib.optionalAttrs (match.absentQueryParams != []) {
      not = map (name: {query.${name} = ["*"];}) match.absentQueryParams;
    };

  proxyHandler = {
    endpoint,
    hostname,
  }:
    if endpoint.transport == "tcp"
    then throw "canix-toolbelt Caddy registry: TCP endpoint `${endpoint.name}` cannot back an HTTP proxy action"
    else
      {
        handler = "reverse_proxy";
        upstreams = [{dial = "${endpoint.address}:${toString endpoint.port}";}];
      }
      // lib.optionalAttrs (endpoint.transport == "https") {
        transport = {
          protocol = "http";
          tls.server_name =
            if endpoint.tlsServerName != null
            then endpoint.tlsServerName
            else hostname;
        };
      }
      // lib.optionalAttrs (endpoint.transport == "h2c") {
        transport = {
          protocol = "http";
          versions = ["h2c" "2"];
        };
      };

  actionHandlers = {
    action,
    endpoint ? null,
    hostname,
    staticRoots,
  }:
    if action.type == "proxy"
    then
      lib.optional (action.stripPrefix != null) {
        handler = "rewrite";
        strip_path_prefix = action.stripPrefix;
      }
      ++ [
        (proxyHandler {inherit endpoint hostname;})
      ]
    else if action.type == "files"
    then let
      root = staticRoots.${action.rootRef} or (throw "canix-toolbelt Caddy registry: static root `${action.rootRef}` required by `${hostname}` is not configured");
    in [
      {
        handler = "vars";
        inherit root;
      }
      {
        handler = "file_server";
        index_names = action.indexNames;
      }
    ]
    else if action.type == "redirect"
    then [
      {
        handler = "static_response";
        status_code = action.status;
        headers.Location = [
          (
            if action.preserveUri
            then "${action.to}{http.request.uri}"
            else action.to
          )
        ];
      }
    ]
    else if action.type == "respond"
    then [
      ({
          handler = "static_response";
          status_code = action.status;
        }
        // lib.optionalAttrs (action.body != null) {body = action.body;})
    ]
    else throw "canix-toolbelt Caddy registry: unsupported HTTP action `${action.type}`";
in rec {
  mkHttpRoute = {
    hostname,
    route,
    endpoint ? null,
    staticRoots ? {},
  }: {
    match = [(requestMatch hostname route.match)];
    handle =
      lib.optional (route.authPolicy != null) {
        handler = "authenticator";
        portal_name = route.authPolicy;
        route_matcher = "*";
      }
      ++ lib.optional (route.responseHeaders != {}) {
        handler = "headers";
        response.set = route.responseHeaders;
      }
      ++ actionHandlers {
        inherit (route) action;
        inherit endpoint hostname staticRoots;
      };
  };

  mkRelayRoute = {
    endpoint,
    hostname,
  }:
    mkHttpRoute {
      inherit endpoint hostname;
      route = {
        match = {
          paths = [];
          absentQueryParams = [];
        };
        action = {
          type = "proxy";
          endpoint = endpoint.name;
          stripPrefix = null;
        };
        authPolicy = null;
        responseHeaders = {};
      };
    };

  mkRedirectRoute = {
    from,
    to,
    status ? 301,
    preservePath ? true,
  }:
    mkHttpRoute {
      hostname = from;
      route = {
        match = {
          paths = [];
          absentQueryParams = [];
        };
        action = {
          type = "redirect";
          inherit to status;
          preserveUri = preservePath;
        };
        authPolicy = null;
        responseHeaders = {};
      };
    };
}
