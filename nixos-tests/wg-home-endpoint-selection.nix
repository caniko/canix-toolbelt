{pkgs}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  evaluated = import "${pkgs.path}/nixos/lib/eval-config.nix" {
    inherit (pkgs.stdenv.hostPlatform) system;
    modules = [
      ../modules/nixos/registry/hosts.nix
      ../modules/nixos/networking/wg-home-client.nix
      {
        networking.hostName = "client";
        system.stateVersion = "25.11";

        canix-toolbelt = {
          hosts = {
            client = {
              deviceType = "laptop";
              wgHomeIp = "198.51.100.2";
            };
            server = {
              deviceType = "server";
              wgHomeIp = "198.51.100.1";
              wgHomePublicKey = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
            };
          };
          networking = {
            links.wg-home = {
              cidr = "198.51.100.0/24";
              endpointHost = "wg.example.test";
              port = 54321;
            };
            wg-home-client = {
              enableDns = false;
              privateKeyFile = "/run/secrets/wg-home";
              ipv6RetryCooldownSeconds = 900;
              dynamicEndpointRefreshRestartSeconds = 30; # Legacy option remains accepted.
              additionalAllowedIPs = ["192.0.2.1/32"];
              lanEndpoints = ["192.0.2.31"];
            };
          };
        };
      }
    ];
  };

  peer = builtins.head evaluated.config.networking.wireguard.interfaces.wg-home.peers;
  service = evaluated.config.systemd.services.wg-home-endpoint;
  timer = evaluated.config.systemd.timers.wg-home-endpoint;
  selector = service.serviceConfig.ExecStart;
in
  mkEvalCheck {
    name = "wg-home-endpoint-selection";
    resultMessage = "wg-home endpoint selection scenarios passed";
    nativeBuildInputs = [pkgs.coreutils pkgs.gnugrep];
    assertions = [
      {
        name = "nixpkgs-refresh-disabled";
        assertion = peer.endpoint == null && peer.dynamicEndpointRefreshSeconds == 0;
        message = "the peer must start without hostname resolution or nixpkgs refresh";
      }
      {
        name = "additional-routes-are-declarative";
        assertion = peer.allowedIPs == ["198.51.100.0/24" "192.0.2.1/32"];
        message = "additional destination CIDRs must be appended to the VPN CIDR";
      }
      {
        name = "additional-routes-use-wireguard-routes";
        assertion = evaluated.config.networking.wireguard.interfaces.wg-home.allowedIPsAsRoutes;
        message = "additional destination CIDRs must use NixOS WireGuard route lifecycle";
      }
      {
        name = "selector-owns-persistent-state";
        assertion = service.serviceConfig.StateDirectory == "wg-home-endpoint";
        message = "the selector must persist the last endpoint and IPv6 failure state";
      }
      {
        name = "selector-uses-direct-resolver";
        assertion = service.environment.DNS_RESOLVER == "1.1.1.1";
        message = "the selector must query a direct public resolver";
      }
      {
        name = "selector-timer-wired";
        assertion = timer.timerConfig.Unit == "wg-home-endpoint.service" && timer.timerConfig.OnUnitInactiveSec == "10s";
        message = "one timer must own recurring endpoint selection";
      }
      {
        name = "selector-is-oneshot-without-restart";
        assertion = service.serviceConfig.Type == "oneshot" && !(service.serviceConfig ? Restart) && !(service.serviceConfig ? RestartSec);
        message = "the timer, not the oneshot service, must drive retries";
      }
      {
        name = "selector-timer-arms-at-boot";
        assertion = timer.timerConfig.OnBootSec == "5s";
        message = "the timer must self-arm at boot, independent of prior selector activation (OnUnitInactiveSec never fires for a never-activated unit)";
      }
    ];
    runtimeScript = ''
      fake="$TMPDIR/fake"
      mkdir -p "$fake"

      cat > "$fake/dig" <<'EOF'
      #!${pkgs.runtimeShell}
      record="$6"
      case "$CASE:$record" in
        dns-failure:*|restore-set-failed:*|lan-roamed:*|dns-offline*:*|lan-unreachable:*|last-endpoint-unreadable:*|last-endpoint-symlink:*) exit 9 ;;
        dig-usage-error:*) exit 1 ;;
        dig-exec-failed:*) exit 127 ;;
        cname:AAAA) printf '%s\n' 'alias.example.test.' ;;
        cname:A) printf '%s\n' '192.0.2.42.' '198.51.100.10' ;;
        *:AAAA) printf '%s\n' '2001:db8::10' ;;
        *:A) printf '%s\n' '198.51.100.10' ;;
      esac
      EOF

      cat > "$fake/ip" <<'EOF'
      #!${pkgs.runtimeShell}
      if [ "$1" = route ] && [ "$2" = get ]; then
        case "$CASE" in
          lan-onlink) printf '%s\n' "$3 dev eth0 src 192.0.2.20" ;;
          lan-unreachable) printf '%s\n' 'RTNETLINK answers: Network is unreachable' >&2; exit 2 ;;
          lan-route-permission-denied) printf '%s\n' 'RTNETLINK answers: Operation not permitted' >&2; exit 2 ;;
          lan-route-syntax-error) printf '%s\n' 'Invalid route query' >&2; exit 1 ;;
          *) printf '%s\n' "$3 via 10.0.0.1 dev tun0" ;;
        esac
        exit 0
      fi
      if [ "$1" = -6 ] && [ "$2" = route ] && [ "$3" = show ] && [ "$4" = default ]; then
        case "$CASE" in
          no-v6-route|offline-no-v6-route) ;;
          ipv6-route-permission-denied) printf '%s\n' 'RTNETLINK answers: Operation not permitted' >&2; exit 2 ;;
          *) printf '%s\n' 'default via fe80::1 dev eth0' ;;
        esac
        exit 0
      fi
      if [ "$1" = -6 ] && [ "$2" = -o ] && [ "$3" = address ] && [ "$4" = show ]; then
        if [ "$CASE" = ipv6-address-permission-denied ]; then
          printf '%s\n' 'RTNETLINK answers: Operation not permitted' >&2
          exit 2
        fi
        printf '2: %s inet6 2001:db8::20/64 scope global\n' "$NETWORK_SIGNATURE"
        exit 0
      fi
      if [ "$2" = address ] && [ "$3" = show ] && [ "$4" = to ]; then
        case "$CASE:$1" in
          candidate-permission-denied:-6) printf '%s\n' 'RTNETLINK answers: Operation not permitted' >&2; exit 2 ;;
          candidate-syntax-error:-6) printf '%s\n' 'Invalid address query' >&2; exit 1 ;;
        esac
        # Deliberately accept every value: the selector itself must remove CNAMEs.
        exit 0
      fi
      exit 1
      EOF

      cat > "$fake/wg" <<'EOF'
      #!${pkgs.runtimeShell}
      if [ "$1" = show ]; then
        if [ "$CASE" = show-failed ] || { [ "$CASE" = show-failed-after-set ] && [ -s "$TEST_ROOT/wg.log" ]; }; then
          printf '%s\n' 'Operation not permitted' >&2
          exit 1
        fi
        value='(none)'
        case "$3" in
          endpoints) file="$TEST_ROOT/current-endpoint" ;;
          latest-handshakes) file="$TEST_ROOT/handshake"; value=0 ;;
          *) exit 1 ;;
        esac
        if [ -f "$file" ]; then IFS= read -r value < "$file"; fi
        printf '%s\t%s\n' "$WG_PUBLIC_KEY" "$value"
        exit 0
      fi
      if [ "$1" = set ] && [ "$5" = endpoint ]; then
        case "$CASE:$6" in
          restore-set-failed:*|all-candidate-sets-failed:*|recover-set-failed:\[*|mixed-candidate-set-failed:\[*) exit 1 ;;
        esac
        printf '%s\n' "$6" >> "$TEST_ROOT/wg.log"
        printf '%s\n' "$6" > "$TEST_ROOT/current-endpoint"
        exit 0
      fi
      exit 1
      EOF

      cat > "$fake/ping" <<'EOF'
      #!${pkgs.runtimeShell}
      IFS= read -r endpoint < "$TEST_ROOT/current-endpoint"
      case "$CASE:$endpoint" in
        failed-v6:\[*|cooldown-rename-failed:\[*) ;;
        offline:*|offline-no-v6-route:*|mixed-candidate-set-failed:*)
          printf '%s\n' '1 packets transmitted, 0 received, 100% packet loss'
          exit 1
          ;;
        ping-no-route:*) printf '%s\n' 'ping: connect: Network is unreachable' >&2; exit 2 ;;
        ping-transient:*) printf '%s\n' 'ping: connect: Network is unreachable' >&2; exit 1 ;;
        ping-permission-denied:*) printf '%s\n' 'ping: socket: Operation not permitted' >&2; exit 2 ;;
        ping-send-permission-denied:*) printf '%s\n' 'ping: sendmsg: Operation not permitted' >&2; exit 1 ;;
        ping-local-error:*) printf '%s\n' 'ping: local error: message too long, mtu=68' >&2; exit 1 ;;
        ping-local-error-stdout:*) printf '%s\n' 'ping: local error: message too long, mtu=68'; exit 1 ;;
        ping-unexpected-error:*) printf '%s\n' 'ping: unexpected diagnostic' >&2; exit 1 ;;
        *) printf '%s\n' 1000 > "$TEST_ROOT/handshake" ;;
      esac
      EOF

      cat > "$fake/date" <<'EOF'
      #!${pkgs.runtimeShell}
      printf '%s\n' 1000
      EOF

      cat > "$fake/sleep" <<'EOF'
      #!${pkgs.runtimeShell}
      exit 0
      EOF

      cat > "$fake/mv" <<'EOF'
      #!${pkgs.runtimeShell}
      case "$CASE:$3" in
        state-rename-failed:*/last-endpoint|cooldown-rename-failed:*/failed-ipv6)
          printf '%s\n' 'Mocked rename failure' >&2
          exit 1
          ;;
      esac
      exec ${pkgs.coreutils}/bin/mv "$@"
      EOF

      cat > "$fake/stat" <<'EOF'
      #!${pkgs.runtimeShell}
      path="''${!#}"
      if [ "$path" = "$STATE_DIRECTORY/last-endpoint" ]; then
        case "$CASE" in
          state-stat-eio) diagnostic='Input/output error' ;;
          state-stat-eacces) diagnostic='Permission denied' ;;
          *) exec ${pkgs.coreutils}/bin/stat "$@" ;;
        esac
        printf "stat: cannot statx '%s': %s\n" "$path" "$diagnostic" >&2
        exit 1
      fi
      exec ${pkgs.coreutils}/bin/stat "$@"
      EOF

      cat > "$fake/cat-state" <<'EOF'
      #!${pkgs.runtimeShell}
      : > "$TEST_ROOT/cat-called"
      exec ${pkgs.coreutils}/bin/cat "$@"
      EOF

      chmod +x "$fake"/*
      export PATH="$fake:$PATH"

      run_case() {
        CASE="$1"
        shift
        NETWORK_SIGNATURE=stable-network
        TEST_ROOT="$TMPDIR/$CASE"
        STATE_DIRECTORY="$TEST_ROOT/state"
        export CASE NETWORK_SIGNATURE TEST_ROOT STATE_DIRECTORY
        mkdir -p "$STATE_DIRECTORY"
        : > "$TEST_ROOT/wg.log"

        case "$CASE" in
          dns-failure|restore-set-failed|state-stat-eio|state-stat-eacces)
            printf '%s' '198.51.100.9:54321' > "$STATE_DIRECTORY/last-endpoint"
            ;;
          promotion)
            printf '%s\n' '198.51.100.10:54321' > "$TEST_ROOT/current-endpoint"
            printf '%s\n' 990 > "$TEST_ROOT/handshake"
            ;;
          lan-roamed|lan-unreachable)
            printf '%s\n' '192.0.2.31:54321' > "$STATE_DIRECTORY/last-endpoint"
            printf '%s\n' '192.0.2.31:54321' > "$TEST_ROOT/current-endpoint"
            ;;
          state-mkdir-failed)
            rmdir "$STATE_DIRECTORY"
            : > "$STATE_DIRECTORY"
            ;;
          state-write-failed)
            ln -s /dev/full "$STATE_DIRECTORY/.last-endpoint"
            ;;
          dns-offline-empty-state)
            : > "$STATE_DIRECTORY/last-endpoint"
            ;;
          last-endpoint-unreadable)
            mkdir "$STATE_DIRECTORY/last-endpoint"
            ;;
          last-endpoint-symlink)
            printf '%s\n' '198.51.100.9:54321' > "$TEST_ROOT/state-target"
            ln -s "$TEST_ROOT/state-target" "$STATE_DIRECTORY/last-endpoint"
            ;;
          cooldown-unreadable)
            mkdir "$STATE_DIRECTORY/failed-ipv6"
            ;;
        esac

        env "$@" "${selector}" > "$TEST_ROOT/output" 2> "$TEST_ROOT/stderr"
      }

      expect_failure() {
        local scenario="$1" diagnostic="$2"
        shift 2
        if run_case "$scenario" "$@"; then
          cat "$TEST_ROOT/output" "$TEST_ROOT/stderr"
          printf 'Expected %s to fail\n' "$scenario" >&2
          exit 1
        fi
        grep -F "$diagnostic" "$TEST_ROOT/stderr"
      }

      export DIG="$fake/dig"
      export IP="$fake/ip"
      export WG="$fake/wg"
      export PING="$fake/ping"
      export DATE="$fake/date"
      export SLEEP="$fake/sleep"
      export SHA256SUM="${pkgs.coreutils}/bin/sha256sum"
      export HANDSHAKE_WAIT_SECONDS=1
      export DNS_RESOLVER=${lib.escapeShellArg service.environment.DNS_RESOLVER}
      export ENDPOINT_HOST=${lib.escapeShellArg service.environment.ENDPOINT_HOST}
      export IPV6_RETRY_COOLDOWN_SECONDS=${lib.escapeShellArg service.environment.IPV6_RETRY_COOLDOWN_SECONDS}
      export REFRESH_SECONDS=${lib.escapeShellArg service.environment.REFRESH_SECONDS}
      export WG_INTERFACE=${lib.escapeShellArg service.environment.WG_INTERFACE}
      export WG_PORT=${lib.escapeShellArg service.environment.WG_PORT}
      export WG_PUBLIC_KEY=${lib.escapeShellArg service.environment.WG_PUBLIC_KEY}
      export WG_SERVER_VPN_IP=${lib.escapeShellArg service.environment.WG_SERVER_VPN_IP}
      export LAN_ENDPOINTS=${lib.escapeShellArg service.environment.LAN_ENDPOINTS}

      run_case lan-onlink
      grep -Fx '192.0.2.31:54321' "$TEST_ROOT/current-endpoint"
      grep -Fx '192.0.2.31:54321' "$STATE_DIRECTORY/last-endpoint"

      run_case lan-offlink
      if grep -Fq '192.0.2.31:54321' "$TEST_ROOT/wg.log"; then
        echo 'Tried an off-link LAN endpoint' >&2
        exit 1
      fi
      grep -Fx '[2001:db8::10]:54321' "$TEST_ROOT/current-endpoint"

      for scenario in dns-offline dns-offline-empty-state lan-roamed lan-unreachable; do
        run_case "$scenario"
        test ! -s "$TEST_ROOT/wg.log"
        grep -F 'fallback=dns-failure no-usable-endpoint action=defer-to-timer' "$TEST_ROOT/output"
      done
      grep -Fx 'RTNETLINK answers: Network is unreachable' "$TEST_ROOT/stderr"
      test ! -s "$TMPDIR/dns-offline/stderr"

      for scenario in offline offline-no-v6-route; do
        run_case "$scenario"
        grep -F 'fallback=no-working-candidate(' "$TEST_ROOT/output" | grep -F 'no-usable-endpoint action=defer-to-timer'
      done
      grep -F 'fallback=no-working-candidate(no-ipv6-default-route)' "$TEST_ROOT/output"

      expect_failure show-failed 'error=wg-show-failed'
      grep -F 'Operation not permitted' "$TEST_ROOT/stderr"
      expect_failure show-failed-after-set 'error=wg-show-failed'
      expect_failure ping-permission-denied 'error=ping-failed'
      expect_failure ping-no-route 'error=ping-failed'
      expect_failure ping-send-permission-denied 'error=ping-failed'
      grep -Fx 'ping: sendmsg: Operation not permitted' "$TEST_ROOT/stderr"
      for scenario in ping-local-error ping-local-error-stdout; do
        expect_failure "$scenario" 'error=ping-failed'
        grep -Fx 'ping: local error: message too long, mtu=68' "$TEST_ROOT/stderr"
      done
      expect_failure ping-unexpected-error 'error=ping-failed'
      grep -Fx 'ping: unexpected diagnostic' "$TEST_ROOT/stderr"
      run_case ping-transient
      grep -Fx 'ping: connect: Network is unreachable' "$TEST_ROOT/stderr"
      grep -F 'no-usable-endpoint action=defer-to-timer' "$TEST_ROOT/output"
      expect_failure restore-set-failed 'error=wg-set-failed'
      if grep -Fq 'retained=true' "$TEST_ROOT/output"; then exit 1; fi
      run_case mixed-candidate-set-failed
      grep -F 'action=probe candidate=[2001:db8::10]:54321 status=1' "$TEST_ROOT/stderr"
      grep -Fx '198.51.100.10:54321' "$TEST_ROOT/current-endpoint"
      grep -F 'no-usable-endpoint action=defer-to-timer' "$TEST_ROOT/output"
      test ! -e "$STATE_DIRECTORY/last-endpoint"
      test ! -e "$STATE_DIRECTORY/failed-ipv6"
      expect_failure all-candidate-sets-failed 'error=wg-set-failed family=none'
      grep -F 'action=probe candidate=[2001:db8::10]:54321 status=1' "$TEST_ROOT/stderr"
      grep -F 'action=probe candidate=198.51.100.10:54321 status=1' "$TEST_ROOT/stderr"
      test ! -e "$STATE_DIRECTORY/failed-ipv6"
      run_case recover-set-failed
      grep -Fx '198.51.100.10:54321' "$STATE_DIRECTORY/last-endpoint"
      for scenario in lan-route-permission-denied candidate-permission-denied ipv6-route-permission-denied ipv6-address-permission-denied; do
        expect_failure "$scenario" 'RTNETLINK answers: Operation not permitted'
      done
      expect_failure lan-route-syntax-error 'Invalid route query'
      expect_failure candidate-syntax-error 'Invalid address query'
      expect_failure state-mkdir-failed 'error=state-mkdir-failed'
      expect_failure last-endpoint-unreadable 'error=state-read-failed path='
      expect_failure cooldown-unreadable 'error=state-read-failed path='
      expect_failure last-endpoint-symlink 'detail=not a regular file (symbolic link)' CAT="$fake/cat-state"
      test ! -e "$TEST_ROOT/cat-called"
      expect_failure state-stat-eio 'error=state-read-failed path=' STAT="$fake/stat"
      grep -Fx "stat: cannot statx '$STATE_DIRECTORY/last-endpoint': Input/output error" "$TEST_ROOT/stderr"
      expect_failure state-stat-eacces 'error=state-read-failed path=' STAT="$fake/stat"
      grep -Fx "stat: cannot statx '$STATE_DIRECTORY/last-endpoint': Permission denied" "$TEST_ROOT/stderr"
      expect_failure state-write-failed 'error=state-write-failed'
      test ! -e "$STATE_DIRECTORY/.last-endpoint"
      test ! -L "$STATE_DIRECTORY/.last-endpoint"
      expect_failure state-rename-failed 'error=state-write-failed'
      test ! -e "$STATE_DIRECTORY/.last-endpoint"
      expect_failure cooldown-rename-failed 'error=state-write-failed'
      test ! -e "$STATE_DIRECTORY/.failed-ipv6"
      expect_failure dig-usage-error 'status=1'
      expect_failure dig-exec-failed 'status=127'
      for key in DNS_RESOLVER WG_SERVER_VPN_IP; do
        expect_failure "unset-$key" "error=missing-config variable=$key" -u "$key"
        expect_failure "empty-$key" "error=missing-config variable=$key" "$key="
      done
      old_ping="$PING"
      PING="$fake/missing"
      expect_failure missing-command 'error=command-unavailable command='
      grep -F 'status=127' "$TEST_ROOT/stderr"
      PING="$fake/not-executable"
      : > "$PING"
      expect_failure nonexecutable-command 'status=126'
      PING="$old_ping"

      run_case reachable-v6
      grep -Fx '[2001:db8::10]:54321' "$TEST_ROOT/current-endpoint"
      grep -F 'family=IPv6 candidate=2001:db8::10 handshake=1000 selected=true' "$TEST_ROOT/output"

      run_case failed-v6
      grep -Fx '198.51.100.10:54321' "$TEST_ROOT/current-endpoint"
      grep -F 'family=IPv6 candidate=2001:db8::10' "$TEST_ROOT/output"
      grep -F 'family=IPv4 candidate=198.51.100.10 handshake=1000 selected=true fallback=handshake-not-newer' "$TEST_ROOT/output"
      "${selector}" > "$TEST_ROOT/output-second"
      [ "$(grep -Fc '[2001:db8::10]:54321' "$TEST_ROOT/wg.log")" -eq 1 ]
      grep -F 'fallback=ipv6-retry-cooldown' "$TEST_ROOT/output-second"
      export NETWORK_SIGNATURE=changed-network
      "${selector}" > "$TEST_ROOT/output-changed-network"
      [ "$(grep -Fc '[2001:db8::10]:54321' "$TEST_ROOT/wg.log")" -eq 2 ]

      run_case no-v6-route
      grep -Fx '198.51.100.10:54321' "$TEST_ROOT/current-endpoint"
      if grep -Fq '[2001:db8::10]:54321' "$TEST_ROOT/wg.log"; then
        echo 'Tried IPv6 without a default route' >&2
        exit 1
      fi
      grep -F 'fallback=no-ipv6-default-route' "$TEST_ROOT/output"

      run_case dns-failure
      grep -Fx '198.51.100.9:54321' "$TEST_ROOT/current-endpoint"
      grep -F 'fallback=dns-failure' "$TEST_ROOT/output"

      run_case cname
      grep -Fx '198.51.100.10:54321' "$TEST_ROOT/current-endpoint"
      if grep -Eq 'alias\.example\.test|192\.0\.2\.42\.' "$TEST_ROOT/wg.log"; then
        echo 'Tried a CNAME as a literal endpoint' >&2
        exit 1
      fi

      run_case promotion
      grep -Fx '[2001:db8::10]:54321' "$TEST_ROOT/current-endpoint"
      grep -F 'family=IPv6 candidate=2001:db8::10 handshake=1000 selected=true' "$TEST_ROOT/output"
    '';
  }
