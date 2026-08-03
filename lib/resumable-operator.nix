{lib}: {
  pkgs,
  name,
  stages,
  stateDir,
  requestPath ? "${stateDir}/requested",
  sentinelPath ? "${stateDir}/running",
  maxAttempts ? 1,
  retryDelays ? [],
  quiesceUnits ? [],
  resumeOnBoot ? false,
  description ? "${name} resumable operator",
  after ? [],
  requires ? [],
  wants ? [],
  extraServiceConfig ? {},
}:
assert stages != [];
assert maxAttempts > 0;
assert builtins.length retryDelays >= maxAttempts - 1; let
  stageRecords = lib.imap0 (index: stage: stage // {inherit index;}) stages;
  stageScript = builtins.concatStringsSep "\n" (builtins.map (stage: ''
      run_stage ${lib.escapeShellArg stage.name} ${lib.escapeShellArg stage.unit} ${toString stage.index} || true
    '')
    stageRecords);
  retryDelayArray = lib.concatMapStringsSep " " lib.escapeShellArg retryDelays;
  workerArray = lib.concatMapStringsSep " " lib.escapeShellArg quiesceUnits;
  controller = pkgs.writeShellApplication {
    name = "${name}-controller";
    runtimeInputs = with pkgs; [coreutils jq systemd util-linux];
    text = ''
      set -u

      state_dir=${lib.escapeShellArg stateDir}
      request_path=${lib.escapeShellArg requestPath}
      sentinel_path=${lib.escapeShellArg sentinelPath}
      state_path="$state_dir/state.json"
      stage_dir="$state_dir/stages"
      max_attempts=${toString maxAttempts}
      stage_count=${toString (builtins.length stageRecords)}
      retry_delays=(${retryDelayArray})
      worker_units=(${workerArray})
      current_unit=""
      current_stage=""
      current_index=-1
      overall_failed=0
      terminal=0
      interrupted=0
      restore_workers=()

      mkdir -p "$state_dir/runs" "$stage_dir"

      if [[ "''${1:-}" == "--cancel" ]]; then
        rm -f "$request_path"
        if [[ -f "$state_path" ]]; then
          tmp="$state_path.tmp"
          jq '.state = "cancelled" | .result = "cancelled" | .updated_at = (now | todateiso8601)' \
            "$state_path" > "$tmp" && mv -f "$tmp" "$state_path"
        fi
        echo "cancelled ${name} run"
        exit 0
      fi

      exec 9>"$state_dir/operator.lock"
      if ! flock -n 9; then
        echo "${name} is already running" >&2
        exit 75
      fi

      write_state() {
        local run_state="$1"
        local run_result="$2"
        local tmp="$state_path.tmp"
        jq -n \
          --arg run_id "$run_id" \
          --arg state "$run_state" \
          --arg result "$run_result" \
          --arg current_stage "$current_stage" \
          --arg current_unit "$current_unit" \
          --argjson current_index "$current_index" \
          --argjson stage_count "$stage_count" \
          --arg report "$run_dir/report.json" \
          '{run_id: $run_id, state: $state, result: $result,
            current_stage: $current_stage, current_unit: $current_unit,
            current_index: $current_index, stage_count: $stage_count,
            report: $report, updated_at: (now | todateiso8601)}' \
          > "$tmp" && mv -f "$tmp" "$state_path"
      }

      append_event() {
        local event="$1"
        local stage="$2"
        local unit="$3"
        local attempt="$4"
        local result="$5"
        local exit_status="$6"
        jq -cn \
          --arg event "$event" \
          --arg stage "$stage" \
          --arg unit "$unit" \
          --arg result "$result" \
          --argjson attempt "$attempt" \
          --argjson exit_status "$exit_status" \
          '{event: $event, stage: $stage, unit: $unit,
            attempt: $attempt, result: $result, exit_status: $exit_status,
            at: (now | todateiso8601)}' >> "$run_dir/events.jsonl"
      }

      # shellcheck disable=SC2329 # called from the signal trap below
      stop_current_stage() {
        if [[ -n "$current_unit" ]]; then
          systemctl stop "$current_unit" || true
        fi
      }

      run_stage() {
        local stage="$1"
        local unit="$2"
        local index="$3"
        local done_path="$stage_dir/$index.done"
        local attempt start_status result exit_status

        if [[ -f "$done_path" ]]; then
          append_event skipped "$stage" "$unit" 0 success 0
          return 0
        fi

        current_stage="$stage"
        current_index="$index"
        current_unit="$unit"
        write_state running running
        systemd-notify --status="${name}: $stage ($((index + 1))/$stage_count)" || true
        for ((attempt = 1; attempt <= max_attempts; attempt++)); do
          systemctl reset-failed "$unit" || true
          if systemctl start --wait "$unit"; then
            start_status=0
          else
            start_status=$?
          fi
          result=$(systemctl show -p Result --value "$unit" 2>/dev/null || echo unknown)
          exit_status=$(systemctl show -p ExecMainStatus --value "$unit" 2>/dev/null || echo "$start_status")
          [[ "$exit_status" =~ ^[0-9]+$ ]] || exit_status="$start_status"
          if [[ "$start_status" -eq 0 && "$result" == success ]]; then
            : > "$done_path"
            append_event completed "$stage" "$unit" "$attempt" success "$exit_status"
            current_unit=""
            write_state running running
            return 0
          fi
          append_event attempt_failed "$stage" "$unit" "$attempt" "$result" "$exit_status"
          if ((attempt < max_attempts)); then
            sleep "''${retry_delays[$((attempt - 1))]}"
          fi
        done
        append_event failed "$stage" "$unit" "$max_attempts" failed "$exit_status"
        overall_failed=1
        current_unit=""
        write_state running running
        return 1
      }

      # shellcheck disable=SC2329 # called from the EXIT trap below
      cleanup() {
        local exit_status=$?
        trap - EXIT
        if [[ "$interrupted" -eq 1 ]]; then
          write_state interrupted interrupted || true
        fi
        rm -f "$sentinel_path"
        for unit in "''${restore_workers[@]}"; do
          systemctl start "$unit" || echo "warning: could not restore $unit" >&2
        done
        if [[ "$terminal" -eq 1 ]]; then
          rm -f "$request_path"
        fi
        exit "$exit_status"
      }
      trap cleanup EXIT
      trap 'interrupted=1; stop_current_stage; exit 143' INT TERM

      run_id=""
      run_dir=""
      if [[ -f "$request_path" && -f "$state_path" ]] \
        && jq -e '.state == "running" or .state == "interrupted"' "$state_path" >/dev/null 2>&1; then
        run_id=$(jq -r '.run_id' "$state_path")
        run_dir="$state_dir/runs/$run_id"
        mkdir -p "$run_dir"
        current_index=$(jq -r '.current_index // -1' "$state_path")
        current_stage=$(jq -r '.current_stage // ""' "$state_path")
      else
        run_id=$(date -u +%Y%m%dT%H%M%SZ)
        run_dir="$state_dir/runs/$run_id"
        mkdir -p "$run_dir"
        rm -f "$stage_dir"/*.done
        : > "$run_dir/events.jsonl"
        current_index=-1
        current_stage=""
        current_unit=""
        write_state running running
      fi
      touch "$request_path"
      [[ -f "$run_dir/events.jsonl" ]] || : > "$run_dir/events.jsonl"

      for unit in "''${worker_units[@]}"; do
        if systemctl is-active --quiet "$unit"; then
          restore_workers+=("$unit")
          systemctl stop "$unit" || true
        fi
      done
      install -D -m 0644 /dev/null "$sentinel_path"
      systemd-notify --status="${name} active; $stage_count stages" --ready || true

      ${stageScript}

      jq -s --arg run_id "$run_id" --arg result "$([[ "$overall_failed" -eq 0 ]] && echo succeeded || echo failed)" \
        '{run_id: $run_id, result: $result, events: .}' \
        "$run_dir/events.jsonl" > "$run_dir/report.json"
      terminal=1
      if [[ "$overall_failed" -eq 0 ]]; then
        write_state succeeded succeeded
        exit 0
      fi
      write_state failed failed
      exit 20
    '';
  };
  mainService = {
    inherit description after requires wants;
    unitConfig = {
      RequiresMountsFor = stateDir;
      StartLimitBurst = 5;
      StartLimitIntervalSec = "1h";
    };
    serviceConfig =
      {
        Type = "notify";
        User = "root";
        Group = "root";
        ExecStart = "${controller}/bin/${name}-controller";
        Restart = "on-failure";
        RestartSec = "2min";
        RestartPreventExitStatus = "20";
        NotifyAccess = "all";
        TimeoutStopSec = "30min";
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        NoNewPrivileges = true;
        RestrictAddressFamilies = ["AF_UNIX"];
        ReadWritePaths = [stateDir (builtins.dirOf sentinelPath)];
        UMask = "0027";
      }
      // extraServiceConfig;
  };
  cancelService = {
    description = "Cancel ${description}";
    conflicts = ["${name}.service"];
    before = ["${name}.service"];
    unitConfig.RequiresMountsFor = stateDir;
    serviceConfig = {
      Type = "oneshot";
      ExecStart = "${controller}/bin/${name}-controller --cancel";
    };
  };
  resumeService = {
    description = "Resume an interrupted ${description}";
    wants = ["network-online.target"];
    after = ["network-online.target"] ++ after;
    wantedBy = ["multi-user.target"];
    unitConfig = {
      ConditionPathExists = requestPath;
      RequiresMountsFor = stateDir;
    };
    serviceConfig = {
      Type = "oneshot";
      ExecStart = "${pkgs.systemd}/bin/systemctl start --no-block ${name}.service";
    };
  };
in {
  systemd.services =
    {
      "${name}" = mainService;
      "${name}-cancel" = cancelService;
    }
    // lib.optionalAttrs resumeOnBoot {
      "${name}-resume" = resumeService;
    };
}
