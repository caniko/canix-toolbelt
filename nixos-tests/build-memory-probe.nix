# A checkpointed episode survives sampler termination and is archived as
# interrupted by the next sampler, against a fixture cgroup directory.
{pkgs}: let
  probe = pkgs.lib.getExe (import ../nix/build-memory-probe.nix {inherit pkgs;});
in
  pkgs.runCommand "build-memory-probe" {nativeBuildInputs = [pkgs.jq];} ''
    cg=$PWD/cgroup reports=$PWD/reports
    mkdir "$cg"
    printf 9000000000 > "$cg/memory.current"
    printf 1024 > "$cg/memory.swap.current"
    printf 9000000000 > "$cg/memory.peak"
    printf 'high 1\nmax 0\noom_kill 0\n' > "$cg/memory.events"
    printf 'some total=1000\nfull total=1000\n' > "$cg/memory.pressure"
    printf max > "$cg/memory.high"
    printf 17179869184 > "$cg/memory.max"

    export BUILD_MEMORY_PROBE_CGROUP=$cg BUILD_MEMORY_PROBE_REPORT_DIR=$reports \
      BUILD_MEMORY_PROBE_INTERVAL_SEC=0.05 BUILD_MEMORY_PROBE_BUSY_BYTES=8589934592 \
      BUILD_MEMORY_PROBE_IDLE_BYTES=2147483648 BUILD_MEMORY_PROBE_IDLE_DEBOUNCE_TICKS=3 \
      BUILD_MEMORY_PROBE_HOST=fixture
    snapshot=$reports/active-episode.json

    wait_for() { # wait_for TEST... (5s)
      for _ in $(seq 250); do "$@" && return 0; sleep 0.02; done
      echo "timed out waiting for: $*" >&2
      return 1
    }

    ${probe} & pid=$!
    wait_for test -s "$snapshot"
    jq -e '.status == "running" and .peak_bytes == 9000000000 and .memory_high == "max"
      and has("boot_id") and has("cgroup_inode")' "$snapshot"
    started=$(jq -r .ts_start "$snapshot")
    kill "$pid"; wait "$pid" || true
    test "$(jq -r .ts_start "$snapshot")" = "$started"
    if compgen -G "$reports/episodes-*.jsonl"; then exit 1; fi

    printf 0 > "$cg/memory.current"
    ${probe} & pid=$!
    wait_for test ! -e "$snapshot"
    kill "$pid"; wait "$pid" || true
    jq -e --arg started "$started" '.status == "interrupted" and .ts_start == $started' \
      "$reports"/interrupted-episodes-*.jsonl

    # A memory.max hit alone (memory.current stays idle) starts an episode.
    ${probe} & pid=$!
    sleep 1.2 # distinct start second, so a distinct episode id
    printf 'high 1\nmax 3\noom_kill 0\n' > "$cg/memory.events"
    wait_for test -e "$reports/episodes-$(date -u +%Y-%m).jsonl"
    kill "$pid"; wait "$pid" || true
    jq -e '.max_hits == 3 and has("episode_id")' "$reports"/episodes-*.jsonl
    done_id=$(jq -r .episode_id "$reports"/episodes-*.jsonl)

    # A checkpoint of an already-published episode is dropped, not re-recorded.
    jq -nc --arg id "$done_id" '{status: "running", episode_id: $id}' > "$snapshot"
    ${probe} & pid=$!
    wait_for test ! -e "$snapshot"
    kill "$pid"; wait "$pid" || true
    if grep -qF "$done_id" "$reports"/interrupted-episodes-*.jsonl; then exit 1; fi
    touch "$out"
  ''
