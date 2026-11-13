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

  # Fallbacks share the primary endpoint's transport and TLS server name.
  proxyHandler = {
    endpoint,
    fallbacks ? [],
    hostname,
  }:
    if endpoint.transport == "tcp"
    then throw "canix-toolbelt Caddy registry: TCP endpoint `${endpoint.name}` cannot back an HTTP proxy action"
    else
      {
        handler = "reverse_proxy";
        upstreams = map (upstream: {dial = "${upstream.address}:${toString upstream.port}";}) ([endpoint] ++ fallbacks);
      }
      // lib.optionalAttrs (fallbacks != []) {
        # Strict ordered failover: always prefer the first healthy upstream.
        # A refused or failed connection retries the next upstream within the
        # same request. Caddy does not retry completed responses: a listed
        # unhealthy status is returned once and marks the upstream down for
        # fail_duration, so later requests fail over.
        load_balancing = {
          selection_policy.policy = "first";
          try_duration = "5s";
          try_interval = "250ms";
        };
        health_checks.passive = {
          fail_duration = "30s";
          max_fails = 1;
          unhealthy_status = [502 503 504];
        };
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

  # Mirrors the forward_auth Caddyfile adapter (Caddy v2.11.7,
  # modules/caddyhttp/reverseproxy/forwardauth/caddyfile.go): a reverse_proxy
  # to the provider that rewrites to GET + provider uri, forwards the original
  # method and uri, and lets 2xx responses fall through to the route's real
  # handlers. Other statuses are returned to the client unchanged (fail closed).
  forwardAuthHandler = provider: let
    inherit (provider) endpoint;
  in
    if endpoint.transport == "tcp"
    then throw "canix-toolbelt Caddy registry: TCP endpoint `${endpoint.name}` cannot back a forwardAuth provider"
    else
      {
        handler = "reverse_proxy";
        headers.request.set = {
          "X-Forwarded-Method" = ["{http.request.method}"];
          "X-Forwarded-Uri" = ["{http.request.uri}"];
        };
        rewrite = {
          method = "GET";
          inherit (provider) uri;
        };
        handle_response = [
          {
            # Status class 2, as the adapter's `@good status 2xx` match.
            match.status_code = [2];
            # A no-op handler keeps the fall-through path: an empty route list
            # makes reverse_proxy write the auth response instead.
            routes = [
              {handle = [{handler = "vars";}];}
            ];
          }
        ];
        upstreams = [
          {dial = "${endpoint.address}:${toString endpoint.port}";}
        ];
      }
      // lib.optionalAttrs (endpoint.transport == "https") {
        transport = {
          protocol = "http";
          # An empty tls block enables TLS; SNI defaults to the dial host.
          tls = lib.optionalAttrs (endpoint.tlsServerName != null) {server_name = endpoint.tlsServerName;};
        };
      }
      // lib.optionalAttrs (endpoint.transport == "h2c") {
        transport = {
          protocol = "http";
          versions = ["h2c" "2"];
        };
      };

  # http.handlers.stream_reauth (github.com/caniko/caddy-stream-reauth) polls
  # the same auth endpoint for the lifetime of a hijacked WebSocket stream.
  streamReauthHandler = provider: let
    inherit (provider) endpoint;
    reauth = provider.streamReauth;
    scheme =
      if endpoint.transport == "https"
      then "https"
      else "http";
  in
    {
      handler = "stream_reauth";
      uri = "${scheme}://${endpoint.address}:${toString endpoint.port}${provider.uri}";
    }
    // lib.optionalAttrs ((reauth.interval or null) != null) {inherit (reauth) interval;}
    // lib.optionalAttrs ((reauth.timeout or null) != null) {inherit (reauth) timeout;}
    // lib.optionalAttrs ((provider.copyHeaders or []) != []) {
      copy_headers = provider.copyHeaders;
    };

  actionHandlers = {
    action,
    endpoint ? null,
    fallbacks ? [],
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
        (proxyHandler {inherit endpoint fallbacks hostname;})
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
        // lib.optionalAttrs (action.body != null) {inherit (action) body;})
    ]
    else throw "canix-toolbelt Caddy registry: unsupported HTTP action `${action.type}`";
in rec {
  mkHttpRoute = {
    hostname,
    route,
    endpoint ? null,
    fallbacks ? [],
    staticRoots ? {},
    # Resolved forwardAuth provider; null for caddy-security policies.
    authProvider ? null,
  }: {
    match = [(requestMatch hostname route.match)];
    handle =
      (
        if route.authPolicy == null
        then []
        else if authProvider == null
        then [
          {
            handler = "authenticator";
            portal_name = route.authPolicy;
            route_matcher = "*";
          }
        ]
        else [(forwardAuthHandler authProvider)] ++ lib.optional ((authProvider.streamReauth or null) != null) (streamReauthHandler authProvider)
      )
      ++ lib.optional (route.responseHeaders != {}) {
        handler = "headers";
        response.set = route.responseHeaders;
      }
      ++ actionHandlers {
        inherit (route) action;
        inherit endpoint fallbacks hostname staticRoots;
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
