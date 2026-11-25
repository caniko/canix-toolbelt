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
    set -eu -o pipefail

    log() {
      printf 'wg-home endpoint: %s\n' "$*"
    }

    fail() {
      local status="$1"
      shift
      log "$* status=$status" >&2
      exit "$status"
    }

    STAT="''${STAT-${pkgs.coreutils}/bin/stat}"
    CAT="''${CAT-${pkgs.coreutils}/bin/cat}"
    for variable in WG_INTERFACE WG_PUBLIC_KEY WG_PORT ENDPOINT_HOST WG_SERVER_VPN_IP DNS_RESOLVER \
      STATE_DIRECTORY HANDSHAKE_WAIT_SECONDS IPV6_RETRY_COOLDOWN_SECONDS REFRESH_SECONDS \
      WG IP DIG PING DATE SHA256SUM SLEEP STAT CAT; do
      [ -n "''${!variable:-}" ] || fail 1 "error=missing-config variable=$variable"
    done
    LAN_ENDPOINTS="''${LAN_ENDPOINTS:-}"

    for tool in "$WG" "$IP" "$DIG" "$PING" "$DATE" "$SHA256SUM" "$SLEEP" "$STAT" "$CAT" chmod mkdir mv rm; do
      case "$tool" in
        */*) path="$tool"; [ -e "$path" ] || fail 127 "error=command-unavailable command=$tool" ;;
        *) path="$(type -P -- "$tool")" || fail 127 "error=command-unavailable command=$tool" ;;
      esac
      [ -f "$path" ] && [ -x "$path" ] || fail 126 "error=command-unavailable command=$tool"
    done

    mkdir -p "$STATE_DIRECTORY" || fail "$?" "error=state-mkdir-failed path=$STATE_DIRECTORY"
    chmod 0700 "$STATE_DIRECTORY" || fail "$?" "error=state-chmod-failed path=$STATE_DIRECTORY"

    peer_value() {
      local command="$1" output key value rest
      output="$("$WG" show "$WG_INTERFACE" "$command")" ||
        fail "$?" "error=wg-show-failed interface=$WG_INTERFACE field=$command"
      while read -r key value rest; do
        if [ "$key" = "$WG_PUBLIC_KEY" ]; then
          printf '%s\n' "$value"
          return
        fi
      done <<< "$output"
      fail 1 "error=wg-peer-not-found interface=$WG_INTERFACE field=$command"
    }

    latest_handshake() {
      local value
      value="$(peer_value latest-handshakes)" || exit "$?"
      case "$value" in
        ""|*[!0-9]*) fail 1 "error=wg-invalid-handshake interface=$WG_INTERFACE" ;;
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

    ip_query() {
      local check="$1" diagnostic status
      shift
      # Keep stdout available to the caller and classify only stderr.
      diagnostic="$(LC_ALL=C "$IP" "$@" 2>&1 >&3)" || {
        status=$?
        if [ "$check" = lan-route-check-failed ]; then
          case "$status:$diagnostic" in
            2:"RTNETLINK answers: Network is unreachable"|2:"RTNETLINK answers: No route to host"|2:"RTNETLINK answers: Network is down")
              printf '%s\n' "$diagnostic" >&2
              return 0
              ;;
          esac
        fi
        fail "$status" "error=$check command=$IP args=$* detail=$diagnostic"
      }
      if [ -n "$diagnostic" ]; then printf '%s\n' "$diagnostic" >&2; fi
    } 3>&1

    # Whether the host reaches LAN candidate $1 on a directly connected route
    # out of a non-tunnel interface, i.e. it is on that LAN right now.
    lan_on_link() {
      local route word previous="" interface=""
      ip_query lan-address-check-failed -4 address show to "$1" >/dev/null
      route="$(ip_query lan-route-check-failed route get "$1")" || exit "$?"
      for word in $route; do
        [ "$word" != via ] || return 1
        if [ "$previous" = dev ]; then interface="$word"; fi
        previous="$word"
      done
      case "$interface" in
        ""|"$WG_INTERFACE"|tun*|tap*|wg*|ppp*|lo) return 1 ;;
      esac
    }

    # A LAN endpoint the host can no longer reach directly (it roamed away).
    unusable_lan_endpoint() {
      local candidate
      for candidate in $LAN_ENDPOINTS; do
        if [ "$1" = "$candidate:$WG_PORT" ]; then
          ! lan_on_link "$candidate"
          return $?
        fi
      done
      return 1
    }

    read_state() {
      local path="$STATE_DIRECTORY/$1" file_type stat_err stat_rc=0
      # Check stat's stdout without following symlinks; capture diagnostics separately.
      stat_err="$(
        {
          file_type="$(LC_ALL=C "$STAT" -c %F -- "$path" 2>&3)" || exit "$?"
          case "$file_type" in
            'regular file'|'regular empty file') ;;
            *) printf 'not a regular file (%s)\n' "$file_type"; exit 1 ;;
          esac
        } 3>&1
      )" || stat_rc=$?
      if [ "$stat_rc" -ne 0 ]; then
        if [[ "$stat_err" == *": No such file or directory" ]]; then return 0; fi
        if [ -n "$stat_err" ]; then printf '%s\n' "$stat_err" >&2; fi
        fail "$stat_rc" "error=state-read-failed path=$path detail=$stat_err"
      fi
      "$CAT" -- "$path" || fail "$?" "error=state-read-failed path=$path"
    }

    write_state() {
      local path="$STATE_DIRECTORY/$1" tmp="$STATE_DIRECTORY/.$1" status
      shift
      umask 077
      printf '%s\n' "$@" > "$tmp" && mv -T "$tmp" "$path" || {
        status=$?
        rm -f -- "$tmp" || log "error=state-cleanup-failed path=$tmp" >&2
        fail "$status" "error=state-write-failed path=$path"
      }
    }

    clear_ipv6_failure() {
      rm -f "$STATE_DIRECTORY/failed-ipv6" ||
        fail "$?" "error=state-remove-failed path=$STATE_DIRECTORY/failed-ipv6"
    }

    restore_last_endpoint() {
      local reason="$1" family handshake status=0
      if [ "$any_wg_set_succeeded" -eq 0 ]; then
        status="$wg_set_failure_status"
      fi
      if [ -z "$last_endpoint" ] || unusable_lan_endpoint "$last_endpoint"; then
        handshake="$(latest_handshake)" || exit "$?"
        if [ "$status" -ne 0 ]; then
          fail "$status" "error=wg-set-failed family=none candidate=none handshake=$handshake fallback=$reason no-usable-endpoint"
        fi
        log "family=none candidate=none handshake=$handshake fallback=$reason no-usable-endpoint action=defer-to-timer"
        return 0
      fi
      if [ "$current_endpoint" != "$last_endpoint" ]; then
        "$WG" set "$WG_INTERFACE" peer "$WG_PUBLIC_KEY" endpoint "$last_endpoint" ||
          fail "$?" "error=wg-set-failed action=restore candidate=$last_endpoint fallback=$reason"
      fi
      family="$(endpoint_family "$last_endpoint")"
      handshake="$(latest_handshake)" || exit "$?"
      log "family=$family candidate=$last_endpoint handshake=$handshake fallback=$reason retained=true"
      return "$status"
    }

    resolve_records() {
      local family="$1" record="$2" output candidate status
      RESOLVED=()
      output="$("$DIG" +time=2 +tries=1 +short "@$DNS_RESOLVER" "$ENDPOINT_HOST" "$record")" || {
        status=$?
        # BIND dig uses 9 for no reply; usage and other failures are fatal.
        [ "$status" -eq 9 ] || fail "$status" "error=dns-query-failed command=$DIG record=$record"
        return 1
      }
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
            *.) continue ;; # Absolute CNAMEs can look like IPv4 literals.
            *.*) ;;
            *) continue ;;
          esac
          case "$candidate" in *[!0-9.]*) continue ;; esac
        fi
        # Parse the numeric literal without requiring it to have a route.
        ip_query candidate-address-check-failed "-$family" address show to "$candidate" >/dev/null
        RESOLVED+=("$candidate")
      done <<< "$output"
    }

    probe_ping() (
      local stderr_file output diagnostic line status=0 unexpected_stderr=false
      stderr_file="$("${pkgs.coreutils}/bin/mktemp")" || fail "$?" "error=ping-stderr-create-failed"
      trap 'rm -f -- "$stderr_file"' EXIT
      output="$(LC_ALL=C "$PING" -n -c 1 -W 2 -I "$WG_INTERFACE" "$WG_SERVER_VPN_IP" 2>"$stderr_file")" || status=$?
      [ "$status" -ne 0 ] || return 0

      diagnostic="$(< "$stderr_file")" || fail "$?" "error=ping-stderr-read-failed"
      if [ -n "$diagnostic" ]; then printf '%s\n' "$diagnostic" >&2; fi
      if [ "$status" -ne 1 ]; then
        if [ -z "$diagnostic" ] && [ -n "$output" ]; then printf '%s\n' "$output" >&2; fi
        fail "$status" "error=ping-failed"
      fi
      if [ -n "$diagnostic" ]; then
        while IFS= read -r line; do
          case "$line" in
            *"local error:"*|*"Operation not permitted"*|*"Permission denied"*) unexpected_stderr=true ;;
            *"Network is unreachable"|*"No route to host"|*"Network is down") ;;
            *) unexpected_stderr=true ;;
          esac
        done <<< "$diagnostic"
      fi

      # iputils uses status 1 for both packet loss and some local errors.
      case "$output" in
        *"local error:"*|*"Operation not permitted"*|*"Permission denied"*|*socket:*|*sendmsg:*|*sendto:*)
          printf '%s\n' "$output" >&2
          fail "$status" "error=ping-failed"
          ;;
      esac
      $unexpected_stderr && fail "$status" "error=ping-failed"
      return 0
    )

    try_candidate() {
      local family="$1" candidate="$2" endpoint before after waited timestamp
      if [ "$family" = "IPv6" ]; then
        endpoint="[$candidate]:$WG_PORT"
      else
        endpoint="$candidate:$WG_PORT"
      fi

      before="$(latest_handshake)" || exit "$?"
      waited=0
      while [ "$before" -gt 0 ] && [ "$waited" -lt 2 ]; do
        timestamp="$("$DATE" +%s)" || fail "$?" "error=date-failed"
        [ "$timestamp" -le "$before" ] || break
        "$SLEEP" 1 || fail "$?" "error=sleep-failed"
        before="$(latest_handshake)" || exit "$?"
        waited=$((waited + 1))
      done
      log "family=$family candidate=$candidate handshake=$before action=try"
      "$WG" set "$WG_INTERFACE" peer "$WG_PUBLIC_KEY" endpoint "$endpoint" || {
        wg_set_failure_status=$?
        log "error=wg-set-failed action=probe candidate=$endpoint status=$wg_set_failure_status" >&2
        ATTEMPT_REASON=wg-set-failed
        return 1
      }
      any_wg_set_succeeded=1
      current_endpoint="$endpoint"
      probe_ping || exit "$?"

      waited=0
      while [ "$waited" -le "$HANDSHAKE_WAIT_SECONDS" ]; do
        after="$(latest_handshake)" || exit "$?"
        if [ "$after" -gt "$before" ]; then
          ATTEMPT_ENDPOINT="$endpoint"
          ATTEMPT_HANDSHAKE="$after"
          ATTEMPT_REASON=selected
          return 0
        fi
        "$SLEEP" 1 || fail "$?" "error=sleep-failed"
        waited=$((waited + 1))
      done
      ATTEMPT_REASON="handshake-not-newer(before=$before,after=$after)"
      return 1
    }

    now="$("$DATE" +%s)" || fail "$?" "error=date-failed"
    wg_set_failure_status=0
    any_wg_set_succeeded=0
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
    if unusable_lan_endpoint "$current_endpoint"; then
      current_healthy=false
    fi

    last_endpoint="$(read_state last-endpoint)" || exit "$?"
    last_endpoint="''${last_endpoint%%$'\n'*}"
    failed_ipv6_state="$(read_state failed-ipv6)" || exit "$?"
    if $current_healthy && [ -n "$current_endpoint" ]; then
      last_endpoint="$current_endpoint"
      write_state last-endpoint "$last_endpoint"
    fi

    # On the server's LAN, its LAN address beats any public candidate: a
    # full-tunnel VPN can route the public address and the resolver away.
    for candidate in $LAN_ENDPOINTS; do
      lan_on_link "$candidate" || continue
      if $current_healthy && [ "$current_endpoint" = "$candidate:$WG_PORT" ]; then
        log "family=LAN candidate=$candidate handshake=$current_handshake selected=healthy-existing"
        exit 0
      fi
      if try_candidate IPv4 "$candidate"; then
        write_state last-endpoint "$ATTEMPT_ENDPOINT"
        log "family=LAN candidate=$candidate handshake=$ATTEMPT_HANDSHAKE selected=true"
        exit 0
      fi
      current_healthy=false
      current_handshake="$(latest_handshake)"
      log "family=LAN candidate=$candidate handshake=$current_handshake fallback=$ATTEMPT_REASON"
    done

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
      clear_ipv6_failure
      log "family=IPv6 candidate=$current_endpoint handshake=$current_handshake selected=healthy-existing"
      exit 0
    fi

    attempted_ipv6=false
    ipv6_fallback_reason=no-aaaa
    ipv6_default_route="$(ip_query ipv6-route-check-failed -6 route show default)" || exit "$?"
    if [ "''${#ipv6_candidates[@]}" -gt 0 ] && [ -z "$ipv6_default_route" ]; then
      ipv6_fallback_reason=no-ipv6-default-route
    elif [ "''${#ipv6_candidates[@]}" -gt 0 ]; then
      ipv6_signature="$(
        ip_query ipv6-address-check-failed -6 -o address show scope global \
          | while read -r index interface family address rest; do
            printf '%s %s\n' "$interface" "$address"
          done \
          | "$SHA256SUM" \
          | while read -r hash rest; do printf '%s' "$hash"; done
      )" || fail "$?" "error=ipv6-signature-failed"

      mapfile -t failed_ipv6 <<< "$failed_ipv6_state"
      failed_candidate="''${failed_ipv6[0]:-}"
      failed_signature="''${failed_ipv6[1]:-}"
      failed_at="''${failed_ipv6[2]:-}"
      case "$failed_at" in ""|*[!0-9]*) failed_at=0 ;; esac

      for candidate in "''${ipv6_candidates[@]}"; do
        if [ "$candidate" = "$failed_candidate" ] && [ "$ipv6_signature" = "$failed_signature" ] && [ "$failed_at" -le "$now" ] && [ $((now - failed_at)) -lt "$IPV6_RETRY_COOLDOWN_SECONDS" ]; then
          ipv6_fallback_reason=ipv6-retry-cooldown
          log "family=IPv6 candidate=$candidate handshake=$current_handshake fallback=$ipv6_fallback_reason"
          continue
        fi
        attempted_ipv6=true
        if try_candidate IPv6 "$candidate"; then
          write_state last-endpoint "$ATTEMPT_ENDPOINT"
          clear_ipv6_failure
          log "family=IPv6 candidate=$candidate handshake=$ATTEMPT_HANDSHAKE selected=true"
          exit 0
        fi
        ipv6_fallback_reason="$ATTEMPT_REASON"
        if [ "$ATTEMPT_REASON" != wg-set-failed ]; then
          write_state failed-ipv6 "$candidate" "$ipv6_signature" "$now"
        fi
        current_handshake="$(latest_handshake)"
        log "family=IPv6 candidate=$candidate handshake=$current_handshake fallback=$ipv6_fallback_reason"
      done
    fi

    if $current_healthy && [ "$current_family" = IPv4 ] && ! $attempted_ipv6; then
      log "family=IPv4 candidate=$current_endpoint handshake=$current_handshake selected=healthy-existing fallback=$ipv6_fallback_reason"
      exit 0
    fi

    for candidate in "''${ipv4_candidates[@]}"; do
      if try_candidate IPv4 "$candidate"; then
        write_state last-endpoint "$ATTEMPT_ENDPOINT"
        log "family=IPv4 candidate=$candidate handshake=$ATTEMPT_HANDSHAKE selected=true fallback=$ipv6_fallback_reason"
        exit 0
      fi
      current_handshake="$(latest_handshake)"
      log "family=IPv4 candidate=$candidate handshake=$current_handshake fallback=$ATTEMPT_REASON"
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
      description = "Deprecated and unused. Retries are driven by wg-home-endpoint.timer rather than service Restart=on-failure.";
    };

    additionalAllowedIPs = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "Additional destination CIDRs routed through the wg-home server.";
    };

    lanEndpoints = lib.mkOption {
      # ponytail: IPv4 only; IPv6 needs canonical-form comparison against
      # `wg show endpoints` before a stale LAN endpoint can be recognized.
      type = lib.types.listOf (lib.types.strMatching "[0-9]{1,3}(\\.[0-9]{1,3}){3}");
      default = [];
      description = "Literal IPv4 LAN addresses of the wg-home server. Tried first, and only while a directly connected non-tunnel route reaches them; a LAN endpoint is dropped once the host roams away.";
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
            allowedIPs = [config.canix-toolbelt.networking.links.wg-home.cidr] ++ cfg.additionalAllowedIPs;
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
        DATE = "date";
        DIG = "dig";
        DNS_RESOLVER = "1.1.1.1";
        ENDPOINT_HOST = wgHome.endpointHost;
        HANDSHAKE_WAIT_SECONDS = "10";
        IP = "ip";
        IPV6_RETRY_COOLDOWN_SECONDS = toString cfg.ipv6RetryCooldownSeconds;
        LAN_ENDPOINTS = lib.concatStringsSep " " cfg.lanEndpoints;
        PING = "ping";
        REFRESH_SECONDS = toString cfg.dynamicEndpointRefreshSeconds;
        SHA256SUM = "sha256sum";
        SLEEP = "sleep";
        WG = "wg";
        WG_INTERFACE = "wg-home";
        WG_PORT = toString wgHome.port;
        WG_PUBLIC_KEY = wgServerPublicKey;
        WG_SERVER_VPN_IP = vpnDnsServerForConfig;
      };
      serviceConfig = {
        Type = "oneshot";
        ExecStart = endpointSelector;
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
