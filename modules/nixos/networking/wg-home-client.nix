{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.networking.wg-home-client;
  inherit (config.canix-toolbelt.networking) wgHome;
  hostname = config.networking.hostName;
  hostWgHomeIp = config.canix-toolbelt.hosts.${hostname}.wgHomeIp or null;

  # The wg-home server is identified by convention as the unique host with
  # wgHomePublicKey set in the shared host registry. Changing that convention
  # should be discussed at the toolbelt module boundary.
  wgServerCandidates = lib.filterAttrs (_: h: (h.wgHomePublicKey or null) != null) config.canix-toolbelt.hosts;
  wgServerNames = lib.attrNames wgServerCandidates;
  wgServer =
    if wgServerCandidates == {}
    then null
    else lib.head (lib.attrValues wgServerCandidates);
  vpnDnsServer = wgServer.wgHomeIp or null;
  vpnDnsServerForConfig =
    if vpnDnsServer != null
    then vpnDnsServer
    else "0.0.0.0";
  wgServerPublicKey = wgServer.wgHomePublicKey or "";

  endpointSelector = pkgs.writeShellScript "wg-home-endpoint-selector" ''
    set -u

    DIG="''${DIG:-dig}"
    IP="''${IP:-ip}"
    WG="''${WG:-wg}"
    PING="''${PING:-ping}"
    DATE="''${DATE:-date}"
    SHA256SUM="''${SHA256SUM:-sha256sum}"
    SLEEP="''${SLEEP:-sleep}"
    STATE_DIRECTORY="''${STATE_DIRECTORY:-/var/lib/wg-home-endpoint}"
    HANDSHAKE_WAIT_SECONDS="''${HANDSHAKE_WAIT_SECONDS:-10}"

    mkdir -p "$STATE_DIRECTORY"
    chmod 0700 "$STATE_DIRECTORY"

    log() {
      printf 'wg-home endpoint: %s\n' "$*"
    }

    peer_value() {
      local command="$1" key value rest
      while read -r key value rest; do
        if [ "$key" = "$WG_PUBLIC_KEY" ]; then
          printf '%s\n' "$value"
          return
        fi
      done < <("$WG" show "$WG_INTERFACE" "$command" 2>/dev/null)
    }

    latest_handshake() {
      local value
      value="$(peer_value latest-handshakes)"
      case "$value" in
        ""|*[!0-9]*) printf '0\n' ;;
        *) printf '%s\n' "$value" ;;
      esac
    }

    endpoint_family() {
      case "$1" in
        \[*\]:*) printf 'IPv6\n' ;;
        *:*) printf 'IPv4\n' ;;
        *) printf 'unknown\n' ;;
      esac
    }

    save_last_endpoint() {
      umask 077
      printf '%s\n' "$1" > "$STATE_DIRECTORY/.last-endpoint"
      mv "$STATE_DIRECTORY/.last-endpoint" "$STATE_DIRECTORY/last-endpoint"
    }

    save_ipv6_failure() {
      umask 077
      printf '%s\n%s\n%s\n' "$1" "$2" "$3" > "$STATE_DIRECTORY/.failed-ipv6"
      mv "$STATE_DIRECTORY/.failed-ipv6" "$STATE_DIRECTORY/failed-ipv6"
    }

    restore_last_endpoint() {
      local reason="$1" family
      if [ -z "$last_endpoint" ]; then
        log "family=none candidate=none handshake=$(latest_handshake) fallback=$reason no-persisted-endpoint"
        return 1
      fi
      if [ "$current_endpoint" != "$last_endpoint" ]; then
        "$WG" set "$WG_INTERFACE" peer "$WG_PUBLIC_KEY" endpoint "$last_endpoint"
      fi
      family="$(endpoint_family "$last_endpoint")"
      log "family=$family candidate=$last_endpoint handshake=$(latest_handshake) fallback=$reason retained=true"
    }

    resolve_records() {
      local family="$1" record="$2" output candidate
      RESOLVED=()
      if ! output="$("$DIG" +time=2 +tries=1 +short "@$DNS_RESOLVER" "$ENDPOINT_HOST" "$record")"; then
        return 1
      fi
      while IFS= read -r candidate; do
        [ -n "$candidate" ] || continue
        if [ "$family" = 6 ]; then
          case "$candidate" in
            *:*) ;;
            *) continue ;;
          esac
          case "$candidate" in *[!0-9A-Fa-f:.]*) continue ;; esac
        else
          case "$candidate" in
            *.*) ;;
            *) continue ;;
          esac
          case "$candidate" in *[!0-9.]*) continue ;; esac
        fi
        # Parse the numeric literal without requiring it to have a route.
        if "$IP" "-$family" address show to "$candidate" >/dev/null 2>&1; then
          RESOLVED+=("$candidate")
        fi
      done <<< "$output"
    }

    try_candidate() {
      local family="$1" candidate="$2" endpoint before after waited
      if [ "$family" = "IPv6" ]; then
        endpoint="[$candidate]:$WG_PORT"
      else
        endpoint="$candidate:$WG_PORT"
      fi

      before="$(latest_handshake)"
      waited=0
      while [ "$before" -gt 0 ] && [ "$("$DATE" +%s)" -le "$before" ] && [ "$waited" -lt 2 ]; do
        "$SLEEP" 1
        before="$(latest_handshake)"
        waited=$((waited + 1))
      done
      log "family=$family candidate=$candidate handshake=$before action=try"
      if ! "$WG" set "$WG_INTERFACE" peer "$WG_PUBLIC_KEY" endpoint "$endpoint"; then
        ATTEMPT_REASON=wg-set-failed
        return 1
      fi
      current_endpoint="$endpoint"
      "$PING" -n -c 1 -W 2 -I "$WG_INTERFACE" "$WG_SERVER_VPN_IP" >/dev/null 2>&1 || true

      waited=0
      while [ "$waited" -le "$HANDSHAKE_WAIT_SECONDS" ]; do
        after="$(latest_handshake)"
        if [ "$after" -gt "$before" ]; then
          ATTEMPT_ENDPOINT="$endpoint"
          ATTEMPT_HANDSHAKE="$after"
          ATTEMPT_REASON=selected
          return 0
        fi
        "$SLEEP" 1
        waited=$((waited + 1))
      done
      ATTEMPT_REASON="handshake-not-newer(before=$before,after=$after)"
      return 1
    }

    now="$("$DATE" +%s)"
    healthy_window=$((REFRESH_SECONDS * 3))
    if [ "$healthy_window" -lt 90 ]; then
      healthy_window=90
    fi

    current_endpoint="$(peer_value endpoints)"
    [ "$current_endpoint" != "(none)" ] || current_endpoint=""
    current_handshake="$(latest_handshake)"
    current_healthy=false
    if [ "$current_handshake" -gt 0 ] && [ "$current_handshake" -le "$now" ] && [ $((now - current_handshake)) -le "$healthy_window" ]; then
      current_healthy=true
    fi

    last_endpoint=""
    if [ -f "$STATE_DIRECTORY/last-endpoint" ]; then
      IFS= read -r last_endpoint < "$STATE_DIRECTORY/last-endpoint" || true
    fi
    if $current_healthy && [ -n "$current_endpoint" ]; then
      last_endpoint="$current_endpoint"
      save_last_endpoint "$last_endpoint"
    fi

    ipv6_status=0
    if resolve_records 6 AAAA; then
      ipv6_candidates=("''${RESOLVED[@]}")
    else
      ipv6_status=1
      ipv6_candidates=()
    fi
    ipv4_status=0
    if resolve_records 4 A; then
      ipv4_candidates=("''${RESOLVED[@]}")
    else
      ipv4_status=1
      ipv4_candidates=()
    fi

    if [ "$ipv6_status" -ne 0 ] && [ "$ipv4_status" -ne 0 ]; then
      restore_last_endpoint dns-failure
      exit $?
    fi

    current_family="$(endpoint_family "$current_endpoint")"
    if $current_healthy && [ "$current_family" = IPv6 ]; then
      rm -f "$STATE_DIRECTORY/failed-ipv6"
      log "family=IPv6 candidate=$current_endpoint handshake=$current_handshake selected=healthy-existing"
      exit 0
    fi

    attempted_ipv6=false
    ipv6_fallback_reason=no-aaaa
    ipv6_default_route="$("$IP" -6 route show default 2>/dev/null)"
    if [ "''${#ipv6_candidates[@]}" -gt 0 ] && [ -z "$ipv6_default_route" ]; then
      ipv6_fallback_reason=no-ipv6-default-route
    elif [ "''${#ipv6_candidates[@]}" -gt 0 ]; then
      ipv6_signature="$(
        "$IP" -6 -o address show scope global 2>/dev/null \
          | while read -r index interface family address rest; do
            printf '%s %s\n' "$interface" "$address"
          done \
          | "$SHA256SUM" \
          | while read -r hash rest; do printf '%s' "$hash"; done
      )"

      failed_candidate=""
      failed_signature=""
      failed_at=0
      if [ -f "$STATE_DIRECTORY/failed-ipv6" ]; then
        {
          IFS= read -r failed_candidate || true
          IFS= read -r failed_signature || true
          IFS= read -r failed_at || true
        } < "$STATE_DIRECTORY/failed-ipv6"
      fi
      case "$failed_at" in ""|*[!0-9]*) failed_at=0 ;; esac

      for candidate in "''${ipv6_candidates[@]}"; do
        if [ "$candidate" = "$failed_candidate" ] && [ "$ipv6_signature" = "$failed_signature" ] && [ "$failed_at" -le "$now" ] && [ $((now - failed_at)) -lt "$IPV6_RETRY_COOLDOWN_SECONDS" ]; then
          ipv6_fallback_reason=ipv6-retry-cooldown
          log "family=IPv6 candidate=$candidate handshake=$current_handshake fallback=$ipv6_fallback_reason"
          continue
        fi
        attempted_ipv6=true
        if try_candidate IPv6 "$candidate"; then
          save_last_endpoint "$ATTEMPT_ENDPOINT"
          rm -f "$STATE_DIRECTORY/failed-ipv6"
          log "family=IPv6 candidate=$candidate handshake=$ATTEMPT_HANDSHAKE selected=true"
          exit 0
        fi
        ipv6_fallback_reason="$ATTEMPT_REASON"
        save_ipv6_failure "$candidate" "$ipv6_signature" "$now"
        log "family=IPv6 candidate=$candidate handshake=$(latest_handshake) fallback=$ipv6_fallback_reason"
      done
    fi

    if $current_healthy && [ "$current_family" = IPv4 ] && ! $attempted_ipv6; then
      log "family=IPv4 candidate=$current_endpoint handshake=$current_handshake selected=healthy-existing fallback=$ipv6_fallback_reason"
      exit 0
    fi

    for candidate in "''${ipv4_candidates[@]}"; do
      if try_candidate IPv4 "$candidate"; then
        save_last_endpoint "$ATTEMPT_ENDPOINT"
        log "family=IPv4 candidate=$candidate handshake=$ATTEMPT_HANDSHAKE selected=true fallback=$ipv6_fallback_reason"
        exit 0
      fi
      log "family=IPv4 candidate=$candidate handshake=$(latest_handshake) fallback=$ATTEMPT_REASON"
    done

    restore_last_endpoint "no-working-candidate($ipv6_fallback_reason)"
  '';
in {
  imports = [
    ./wg-home-shared.nix
  ];

  options.canix-toolbelt.networking.wg-home-client = {
    clientIp = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default =
        if hostWgHomeIp != null
        then "${hostWgHomeIp}/24"
        else null;
      description = "The IP address for this client on the wg-home VPN, including prefix length.";
    };

    secretName = lib.mkOption {
      type = lib.types.str;
      default = "${hostname}-wg-home-client-pk";
      defaultText = lib.literalExpression ''"${config.networking.hostName}-wg-home-client-pk"'';
      description = "Name of the age secret containing this host's wg-home private key.";
    };

    privateKeyFile = lib.mkOption {
      type = lib.types.str;
      default = config.age.secrets.${cfg.secretName}.path or "";
      defaultText = lib.literalExpression "config.age.secrets.\${config.canix-toolbelt.networking.wg-home-client.secretName}.path";
      description = "Path to the wg-home private key file.";
    };

    enableDns = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Configure split DNS for the wg-home VPN domain via the VPN DNS server.";
    };

    dynamicEndpointRefreshSeconds = lib.mkOption {
      type = lib.types.ints.positive;
      default = 10;
      description = "Seconds between wg-home endpoint selection checks.";
    };

    dynamicEndpointRefreshRestartSeconds = lib.mkOption {
      type = lib.types.ints.unsigned;
      default = 60;
      description = "Seconds to wait before restarting the wg-home endpoint selection service after failure.";
    };

    ipv6RetryCooldownSeconds = lib.mkOption {
      type = lib.types.ints.positive;
      default = 900;
      description = "Seconds to suppress a failed IPv6 candidate until retrying it on the same IPv6 network.";
    };
  };

  config = lib.mkIf (hostWgHomeIp != null) {
    assertions = [
      {
        assertion = cfg.clientIp != null;
        message = "canix-toolbelt.networking.wg-home-client.clientIp must be set, either directly or via canix-toolbelt.hosts.<hostname>.wgHomeIp";
      }
      {
        assertion = lib.length wgServerNames == 1;
        message = "canix-toolbelt.networking.wg-home-client requires exactly one host with wgHomePublicKey set in canix-toolbelt.hosts";
      }
      {
        assertion = vpnDnsServer != null;
        message = "canix-toolbelt.networking.wg-home-client requires the wg-home server host to have wgHomeIp set";
      }
    ];

    services.resolved.enable = lib.mkIf cfg.enableDns true;

    networking.firewall = {
      allowedUDPPorts = [wgHome.port];
    };

    networking.wireguard.enable = true;
    networking.wireguard.interfaces = {
      wg-home = {
        ips = [cfg.clientIp];
        listenPort = wgHome.port;
        inherit (cfg) privateKeyFile;

        postSetup = lib.mkIf cfg.enableDns ''
          ${pkgs.systemd}/bin/resolvectl dns wg-home ${vpnDnsServerForConfig}
          ${pkgs.systemd}/bin/resolvectl domain wg-home ~${wgHome.vpnDomain}
        '';

        postShutdown = lib.mkIf cfg.enableDns ''
          ${pkgs.systemd}/bin/resolvectl revert wg-home || true
        '';

        peers = [
          {
            publicKey = wgServerPublicKey;
            allowedIPs = [config.canix-toolbelt.networking.links.wg-home.cidr];
            endpoint = null;
            dynamicEndpointRefreshSeconds = 0;
            persistentKeepalive = 25;
          }
        ];
      };
    };

    systemd.services.wg-home-endpoint = {
      description = "Select a working IPv6 or IPv4 wg-home endpoint";
      wantedBy = ["wireguard-wg-home.service"];
      requires = ["wireguard-wg-home.service"];
      after = ["wireguard-wg-home.service" "network-online.target"];
      wants = ["network-online.target"];
      partOf = ["wireguard-wg-home.service"];
      path = with pkgs; [bind.dnsutils coreutils iproute2 iputils wireguard-tools];
      environment = {
        DNS_RESOLVER = "1.1.1.1";
        ENDPOINT_HOST = wgHome.endpointHost;
        HANDSHAKE_WAIT_SECONDS = "10";
        IPV6_RETRY_COOLDOWN_SECONDS = toString cfg.ipv6RetryCooldownSeconds;
        REFRESH_SECONDS = toString cfg.dynamicEndpointRefreshSeconds;
        WG_INTERFACE = "wg-home";
        WG_PORT = toString wgHome.port;
        WG_PUBLIC_KEY = wgServerPublicKey;
        WG_SERVER_VPN_IP = vpnDnsServerForConfig;
      };
      serviceConfig = {
        Type = "oneshot";
        ExecStart = endpointSelector;
        Restart = "on-failure";
        RestartSec = cfg.dynamicEndpointRefreshRestartSeconds;
        StateDirectory = "wg-home-endpoint";
        StateDirectoryMode = "0700";
        PrivateTmp = true;
        ProtectHome = true;
        ProtectSystem = "strict";
      };
    };

    systemd.timers.wg-home-endpoint = {
      description = "Periodically verify the wg-home endpoint";
      wantedBy = ["timers.target"];
      timerConfig = {
        # OnUnitInactiveSec-only timers never fire for a unit that has never
        # been activated (systemd re-arms only on deactivation); the selector
        # therefore never ran on fresh installs and stale endpoints survived
        # a WAN IP change. OnBootSec arms the first run independently.
        OnBootSec = "5s";
        OnUnitInactiveSec = "${toString cfg.dynamicEndpointRefreshSeconds}s";
        Unit = "wg-home-endpoint.service";
      };
    };
  };
}
