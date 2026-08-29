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
              wgHomeIp = "10.123.0.2";
            };
            server = {
              deviceType = "server";
              wgHomeIp = "10.123.0.1";
              wgHomePublicKey = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
            };
          };
          networking = {
            links.wg-home = {
              cidr = "10.123.0.0/24";
              endpointHost = "wg.example.test";
              port = 54321;
            };
            wg-home-client = {
              enableDns = false;
              privateKeyFile = "/run/secrets/wg-home";
              ipv6RetryCooldownSeconds = 900;
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
        assertion = timer.timerConfig.Unit == "wg-home-endpoint.service";
        message = "one timer must own recurring endpoint selection";
      }
      {
        name = "selector-timer-arms-at-boot";
        assertion = timer.timerConfig ? OnBootSec;
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
        dns-failure:*) exit 1 ;;
        cname:AAAA) printf '%s\n' 'alias.example.test.' ;;
        *:AAAA) printf '%s\n' '2001:db8::10' ;;
        *:A) printf '%s\n' '198.51.100.10' ;;
      esac
      EOF

      cat > "$fake/ip" <<'EOF'
      #!${pkgs.runtimeShell}
      if [ "$1" = -6 ] && [ "$2" = route ] && [ "$3" = show ] && [ "$4" = default ]; then
        [ "$CASE" = no-v6-route ] || printf '%s\n' 'default via fe80::1 dev eth0'
        exit 0
      fi
      if [ "$1" = -6 ] && [ "$2" = -o ] && [ "$3" = address ] && [ "$4" = show ]; then
        printf '2: %s inet6 2001:db8::20/64 scope global\n' "$NETWORK_SIGNATURE"
        exit 0
      fi
      if [ "$2" = address ] && [ "$3" = show ] && [ "$4" = to ]; then
        # Deliberately accept every value: the selector itself must remove CNAMEs.
        exit 0
      fi
      exit 1
      EOF

      cat > "$fake/wg" <<'EOF'
      #!${pkgs.runtimeShell}
      if [ "$1" = show ]; then
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
        failed-v6:\[*) ;;
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

      chmod +x "$fake"/*

      run_case() {
        CASE="$1"
        NETWORK_SIGNATURE=stable-network
        TEST_ROOT="$TMPDIR/$CASE"
        STATE_DIRECTORY="$TEST_ROOT/state"
        export CASE NETWORK_SIGNATURE TEST_ROOT STATE_DIRECTORY
        mkdir -p "$STATE_DIRECTORY"
        : > "$TEST_ROOT/wg.log"

        case "$CASE" in
          dns-failure)
            printf '%s\n' '198.51.100.9:54321' > "$STATE_DIRECTORY/last-endpoint"
            ;;
          promotion)
            printf '%s\n' '198.51.100.10:54321' > "$TEST_ROOT/current-endpoint"
            printf '%s\n' 990 > "$TEST_ROOT/handshake"
            ;;
        esac

        "${selector}" > "$TEST_ROOT/output"
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
      ! grep -Fq '[2001:db8::10]:54321' "$TEST_ROOT/wg.log"
      grep -F 'fallback=no-ipv6-default-route' "$TEST_ROOT/output"

      run_case dns-failure
      grep -Fx '198.51.100.9:54321' "$TEST_ROOT/current-endpoint"
      grep -F 'fallback=dns-failure' "$TEST_ROOT/output"

      run_case cname
      grep -Fx '198.51.100.10:54321' "$TEST_ROOT/current-endpoint"
      ! grep -Fq 'alias.example.test' "$TEST_ROOT/wg.log"

      run_case promotion
      grep -Fx '[2001:db8::10]:54321' "$TEST_ROOT/current-endpoint"
      grep -F 'family=IPv6 candidate=2001:db8::10 handshake=1000 selected=true' "$TEST_ROOT/output"
    '';
  }
