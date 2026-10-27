# shellcheck shell=bash
# Run by writeShellApplication (errexit, nounset, pipefail).
# --- configuration (set by the NixOS module's unit environment) ----------
configured_cgroup=${BUILD_MEMORY_PROBE_CGROUP:-}
report_dir=${BUILD_MEMORY_PROBE_REPORT_DIR:?}
interval=${BUILD_MEMORY_PROBE_INTERVAL_SEC:?}
busy=${BUILD_MEMORY_PROBE_BUSY_BYTES:?}
idle=${BUILD_MEMORY_PROBE_IDLE_BYTES:?}
debounce=${BUILD_MEMORY_PROBE_IDLE_DEBOUNCE_TICKS:?}
host=${BUILD_MEMORY_PROBE_HOST:?}
# 1 when Nix places builders in a delegated nix-builders child cgroup.
aggregate_builders=${BUILD_MEMORY_PROBE_AGGREGATE_BUILDERS:-0}

if [ -n "$configured_cgroup" ]; then
  cg="$configured_cgroup"
else
  unit_cgroup=$(systemctl show nix-daemon.service --property=ControlGroup --value 2>/dev/null || true)
  case "$unit_cgroup" in
  /*) cg="/sys/fs/cgroup$unit_cgroup" ;;
  *)
    echo "build-memory-probe: nix-daemon.service has no usable ControlGroup ($unit_cgroup)" >&2
    exit 1
    ;;
  esac
fi

# Nix owns this delegated subtree. Older generations retain the service
# fallback so telemetry can also observe the bootstrap deployment.
if [ "$aggregate_builders" -eq 1 ] && [ -z "$configured_cgroup" ]; then
  # A simple service may be active before Nix finishes delegation. Retry
  # startup instead of silently sampling the service parent all boot.
  if [ ! -r "$cg/nix-builders/memory.current" ]; then
    echo "build-memory-probe: waiting for Nix's delegated builder domain" >&2
    exit 1
  fi
  cg="$cg/nix-builders"
fi

for required in memory.current memory.events memory.pressure memory.high memory.max; do
  if [ ! -r "$cg/$required" ]; then
    echo "build-memory-probe: required cgroup file is unreadable: $cg/$required" >&2
    exit 1
  fi
done

mkdir -p "$report_dir"
boot_id=$(cat /proc/sys/kernel/random/boot_id)
cgroup_inode=$(stat -c %i "$cg")
checkpoint_ticks=0
# Preserve an unfinished episode before a new process overwrites its
# checkpoint. The record cannot attribute why the prior sampler stopped.
if [ -f "$report_dir/active-episode.json" ]; then
  # A crash between publishing the completed record and removing the
  # checkpoint leaves a finished episode behind: do not re-record it.
  stale_id=$(jq -r '.episode_id // empty' "$report_dir/active-episode.json")
  if [ -z "$stale_id" ] || ! grep -qsF "\"episode_id\":\"$stale_id\"" "$report_dir"/episodes-*.jsonl; then
    interrupted="$report_dir/interrupted-episodes-$(date -u +%Y-%m).jsonl"
    jq -c --arg recorded_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
      '. + {status: "interrupted", recorded_at: $recorded_at}' \
      "$report_dir/active-episode.json" >>"$interrupted"
    sync -d "$interrupted"
  fi
  rm "$report_dir/active-episode.json"
  sync -d "$report_dir"
fi

# --- robust cgroup readers (always emit a number / token) -------------
read_int() { # single-int cgroup file; non-numeric (e.g. "max") -> 0
  local f="$1" v
  v=$(cat "$f" 2>/dev/null || echo 0)
  case "$v" in "" | *[!0-9]*) v=0 ;; esac
  printf '%s' "$v"
}
read_token() { # raw token of a cgroup file ("max" or bytes); default given
  local f="$1" d="$2" v
  v=$(cat "$f" 2>/dev/null || echo "$d")
  [ -n "$v" ] || v="$d"
  printf '%s' "$v"
}
read_event() { # memory.events counter by name, 0 if absent
  awk -v k="$1" '$1==k{print $2; f=1} END{if(!f) print 0}' \
    "$cg/memory.events" 2>/dev/null || echo 0
}
read_psi() { # memory.pressure "some"/"full" total= (microseconds)
  awk -v k="$1" '
    $1==k{for(i=2;i<=NF;i++) if($i ~ /^total=/){sub(/total=/,"",$i); print $i; f=1}}
    END{if(!f) print 0}' "$cg/memory.pressure" 2>/dev/null || echo 0
}

echo "build-memory-probe: watching $cg (busy>=$busy idle<$idle every ${interval}s)"

in_episode=0
idle_count=0
start_epoch=0
start_iso=""
sampled_peak=0
base_high=0
base_max=0
base_oom=0
base_psi_some=0
base_psi_full=0
previous_high=$(read_event high)
previous_max=$(read_event max)
previous_oom=$(read_event oom_kill)
previous_psi_full=$(read_psi full)

while true; do
  cur=$(read_int "$cg/memory.current")
  current_high=$(read_event high)
  current_max=$(read_event max)
  current_oom=$(read_event oom_kill)
  current_psi_full=$(read_psi full)

  if [ "$in_episode" -eq 0 ]; then
    if [ "$cur" -ge "$busy" ] || [ "$current_high" -gt "$previous_high" ] ||
      [ "$current_max" -gt "$previous_max" ] ||
      [ "$current_oom" -gt "$previous_oom" ] ||
      [ "$current_psi_full" -gt "$previous_psi_full" ]; then
      in_episode=1
      idle_count=0
      start_epoch=$(date -u +%s)
      start_iso=$(date -u +%Y-%m-%dT%H:%M:%SZ)
      base_high=$previous_high
      base_max=$previous_max
      episode_id="$boot_id-$start_epoch"
      base_oom=$previous_oom
      base_psi_some=$(read_psi some)
      base_psi_full=$previous_psi_full
      sampled_peak=$cur
      sampled_swap_peak=$(read_int "$cg/memory.swap.current")
      echo "build-memory-probe: episode START at $start_iso (current=$cur)"
    fi
  else
    [ "$cur" -gt "$sampled_peak" ] && sampled_peak=$cur
    swap_current=$(read_int "$cg/memory.swap.current")
    [ "$swap_current" -gt "$sampled_swap_peak" ] && sampled_swap_peak=$swap_current

    if [ "$cur" -lt "$idle" ]; then
      idle_count=$((idle_count + 1))
    else
      idle_count=0
    fi

    if [ "$idle_count" -ge "$debounce" ]; then
      end_epoch=$(date -u +%s)
      end_iso=$(date -u +%Y-%m-%dT%H:%M:%SZ)
      dur=$((end_epoch - start_epoch))

      # Kernel peaks are lifetime counters. A write followed by close
      # does not give subsequent cat invocations a per-episode peak.
      peak=$sampled_peak
      swap_peak=$sampled_swap_peak

      high=$(($(read_event high) - base_high))
      maxhits=$(($(read_event max) - base_max))
      oom=$(($(read_event oom_kill) - base_oom))
      psi_some=$(($(read_psi some) - base_psi_some))
      psi_full=$(($(read_psi full) - base_psi_full))

      mhigh=$(read_token "$cg/memory.high" "unknown")
      mmax=$(read_token "$cg/memory.max" "unknown")

      report_file="$report_dir/episodes-$(date -u +%Y-%m).jsonl"

      jq -nc \
        --arg host "$host" \
        --arg episode_id "$episode_id" \
        --arg boot_id "$boot_id" \
        --arg cgroup_inode "$cgroup_inode" \
        --arg cgroup "$cg" \
        --argjson lifetime_peak_bytes "$(read_int "$cg/memory.peak")" \
        --arg ts_start "$start_iso" \
        --arg ts_end "$end_iso" \
        --argjson duration_s "$dur" \
        --argjson peak_bytes "$peak" \
        --argjson swap_peak_bytes "$swap_peak" \
        --argjson throttle_high "$high" \
        --argjson max_hits "$maxhits" \
        --argjson oom_kills "$oom" \
        --argjson psi_some_us "$psi_some" \
        --argjson psi_full_us "$psi_full" \
        --arg memory_high "$mhigh" \
        --arg memory_max "$mmax" \
        '($memory_max | try tonumber catch null) as $mmaxn
         | {
             host: $host,
             episode_id: $episode_id,
             boot_id: $boot_id,
             cgroup_inode: $cgroup_inode,
             cgroup: $cgroup,
             lifetime_peak_bytes: $lifetime_peak_bytes,
             peak_kind: "sampled-episode",
             ts_start: $ts_start,
             ts_end: $ts_end,
             duration_s: $duration_s,
             peak_bytes: $peak_bytes,
             peak_gib: (($peak_bytes / 1073741824) * 100 | round / 100),
             swap_peak_bytes: $swap_peak_bytes,
             swap_peak_gib: (($swap_peak_bytes / 1073741824) * 100 | round / 100),
             throttle_high: $throttle_high,
             max_hits: $max_hits,
             oom_kills: $oom_kills,
             psi_some_stall_s: (($psi_some_us / 1000000) * 100 | round / 100),
             psi_full_stall_s: (($psi_full_us / 1000000) * 100 | round / 100),
             memory_high: $memory_high,
             memory_max: $memory_max,
             hit_ceiling: (if $mmaxn == null then false
                           else ($peak_bytes >= ($mmaxn * 0.99)) end)
           }' >>"$report_file"
      sync -d "$report_file"

      echo "build-memory-probe: episode END dur=${dur}s peak=$peak throttle_high=$high max_hits=$maxhits oom_kills=$oom -> $report_file"
      rm -f "$report_dir/active-episode.json"
      sync -d "$report_dir"

      in_episode=0
      idle_count=0
    fi
  fi

  if [ "$in_episode" -eq 1 ]; then
    # Persist progress before the episode ends: a reboot must not erase
    # the only evidence of a long-running build. The independent incident
    # recorder supplies finer-grained host/parent/handler samples.
    if [ "$checkpoint_ticks" -eq 0 ]; then
      snapshot="$report_dir/active-episode.json"
      jq -nc --arg boot_id "$boot_id" --arg cgroup_inode "$cgroup_inode" \
        --arg host "$host" --arg episode_id "$episode_id" --arg cgroup "$cg" --arg ts_start "$start_iso" \
        --arg ts_observed "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
        --argjson peak_bytes "$sampled_peak" \
        --argjson swap_peak_bytes "$sampled_swap_peak" \
        --arg memory_high "$(read_token "$cg/memory.high" unknown)" \
        --arg memory_max "$(read_token "$cg/memory.max" unknown)" \
        --argjson oom_kills "$((current_oom - base_oom))" \
        --argjson psi_full_us "$((current_psi_full - base_psi_full))" \
        '{status: "running", episode_id: $episode_id, boot_id: $boot_id, cgroup_inode: $cgroup_inode,
          host: $host, cgroup: $cgroup, ts_start: $ts_start, ts_observed: $ts_observed,
          peak_bytes: $peak_bytes, swap_peak_bytes: $swap_peak_bytes,
          memory_high: $memory_high, memory_max: $memory_max,
          oom_kills: $oom_kills, psi_full_us: $psi_full_us}' >"$snapshot.tmp"
      sync -d "$snapshot.tmp"
      mv "$snapshot.tmp" "$snapshot"
      sync -d "$report_dir"
    fi
    checkpoint_ticks=$(((checkpoint_ticks + 1) % 10))
  else
    checkpoint_ticks=0
  fi

  previous_high=$current_high
  previous_max=$current_max
  previous_oom=$current_oom
  previous_psi_full=$current_psi_full

  sleep "$interval"
done
