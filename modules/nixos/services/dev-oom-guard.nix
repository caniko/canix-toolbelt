{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.services.devOomGuard;
  jsonFormat = pkgs.formats.json {};

  matcherSubmodule = {
    options = {
      name = lib.mkOption {
        type = lib.types.str;
        description = "Human-readable label for journal logs.";
      };

      cmdlineRegex = lib.mkOption {
        type = lib.types.str;
        description = "Regex matched against /proc/<pid>/cmdline with NUL separators normalized to spaces.";
      };
    };
  };

  protectSubmodule = {
    options = {
      name = lib.mkOption {
        type = lib.types.str;
        description = "Human-readable label for journal logs.";
      };

      cmdlineRegex = lib.mkOption {
        type = lib.types.str;
        description = "Regex matched against /proc/<pid>/cmdline with NUL separators normalized to spaces.";
      };

      maxAdj = lib.mkOption {
        type = lib.types.ints.between (-1000) 1000;
        default = 0;
        description = ''
          Cap oom_score_adj at this value on every scan. This should stay
          narrow: a broad pattern could undo an application's own OOM policy.
          Negative values are applied by the root-owned protector service;
          -1000 makes the matched process immune to the kernel OOM killer.
        '';
      };

      verifyEditorCgroup = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Validate the matched process's actual cgroup has OOMPolicy=continue
          and memory.oom.group=0. The cgroup is discovered from /proc rather
          than from a compositor-specific unit-name pattern.
        '';
      };
    };
  };

  devOomGuardScript = pkgs.writers.writePython3 "dev-oom-guard" {} ''
    import collections
    import json
    import os
    import pwd
    import re
    import signal
    import subprocess
    import sys
    import time


    SYSTEMD = "${pkgs.systemd}"
    SYSTEMCTL = os.path.join(SYSTEMD, "bin", "systemctl")
    CGROUP_ROOT = "/sys/fs/cgroup"
    running = True
    reported_editor_contract_failures = set()
    paused = {}


    def log(level, message):
        stream = sys.stderr if level in ("WARN", "ERROR", "FATAL") else sys.stdout
        print(f"{level}: {message}", flush=True, file=stream)


    def write_adj(pid, value):
        with open(f"/proc/{pid}/oom_score_adj", "w", encoding="ascii") as f:
            f.write(str(value))


    def read_adj(pid):
        with open(f"/proc/{pid}/oom_score_adj", encoding="ascii") as f:
            return int(f.read().strip())


    def read_cmdline(pid):
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            return f.read().replace(b"\x00", b" ").decode(errors="replace").strip()


    def read_ppid(pid):
        with open(f"/proc/{pid}/status", encoding="ascii") as f:
            for line in f:
                if line.startswith("PPid:"):
                    return int(line.split()[1])
        return 0


    def read_uid(pid):
        with open(f"/proc/{pid}/status", encoding="ascii") as f:
            for line in f:
                if line.startswith("Uid:"):
                    return int(line.split()[1])
        raise RuntimeError(f"cannot find uid for pid={pid}")


    def process_starttime(pid):
        with open(f"/proc/{pid}/stat", encoding="ascii") as f:
            stat = f.read()
        _, fields = stat.rsplit(") ", 1)
        return fields.split()[19]


    def process_state(pid):
        with open(f"/proc/{pid}/status", encoding="ascii") as f:
            for line in f:
                if line.startswith("State:"):
                    return line.split()[1]
        return ""


    def startup_problem(allow_degraded, message):
        if allow_degraded:
            log("WARN", message)
            return
        log("FATAL", message)
        sys.exit(2)


    def cgroup_path_for_pid(pid):
        with open(f"/proc/{pid}/cgroup", encoding="ascii") as f:
            for line in f:
                hierarchy, separator, path = line.rstrip("\n").partition("::")
                if separator and hierarchy == "0":
                    return path or "/"
        raise RuntimeError(f"cannot find unified cgroup for pid={pid}")


    def cgroup_unit_for_path(path):
        for component in reversed(path.strip("/").split("/")):
            if component.endswith((".scope", ".service")):
                return component
        return None


    def manager_default_oom_policy():
        result = subprocess.run(
            [
                SYSTEMCTL,
                "--user",
                "show",
                "--property=DefaultOOMPolicy",
                "--value",
            ],
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        if result.returncode != 0:
            raise RuntimeError(
                "cannot read user-manager DefaultOOMPolicy: "
                f"{result.stderr.strip()}"
            )
        return result.stdout.strip()


    def unit_oom_policy(unit):
        result = subprocess.run(
            [
                SYSTEMCTL,
                "--user",
                "show",
                "--property=OOMPolicy",
                "--value",
                unit,
            ],
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"cannot read OOMPolicy for {unit}: {result.stderr.strip()}"
            )
        return result.stdout.strip()


    def verify_editor_cgroup(pid, allow_degraded):
        path = cgroup_path_for_pid(pid)
        unit = cgroup_unit_for_path(path)
        if unit is None:
            message = f"editor process pid={pid} is not in a systemd unit: {path}"
            if message not in reported_editor_contract_failures:
                reported_editor_contract_failures.add(message)
                log("WARN" if allow_degraded else "ERROR", message)
            return

        oom_group_path = os.path.join(
            CGROUP_ROOT, path.lstrip("/"), "memory.oom.group"
        )
        try:
            with open(oom_group_path, encoding="ascii") as f:
                oom_group = f.read().strip()
        except OSError as e:
            message = (
                f"cannot read editor cgroup memory.oom.group for pid={pid}: {e}"
            )
            if allow_degraded:
                log("WARN", message)
                return
            raise RuntimeError(message) from e

        try:
            oom_policy = unit_oom_policy(unit)
        except RuntimeError as e:
            if allow_degraded:
                log("WARN", str(e))
                return
            raise

        if oom_group != "0" or oom_policy != "continue":
            message = (
                f"editor cgroup contract failed for pid={pid} unit={unit}: "
                f"memory.oom.group={oom_group!r}, OOMPolicy={oom_policy!r}"
            )
            if message not in reported_editor_contract_failures:
                reported_editor_contract_failures.add(message)
                log("WARN" if allow_degraded else "ERROR", message)
            return


    def startup_gate(cfg):
        if cfg.get("verifyEditorScopes", True):
            try:
                policy = manager_default_oom_policy()
                if policy != "continue":
                    startup_problem(
                        cfg.get("allowDegraded", False),
                        "user-manager DefaultOOMPolicy must be continue, "
                        f"got {policy!r}",
                    )
            except Exception as e:
                startup_problem(cfg.get("allowDegraded", False), str(e))


    def read_process_table():
        processes = {}
        children = collections.defaultdict(list)
        for entry in os.scandir("/proc"):
            if not entry.name.isdigit():
                continue
            pid = int(entry.name)
            try:
                ppid = read_ppid(pid)
                uid = read_uid(pid)
                cmdline = read_cmdline(pid)
            except (FileNotFoundError, ProcessLookupError):
                continue
            except PermissionError as e:
                log("WARN", f"cannot inspect pid={pid}: {e}")
                continue
            except OSError:
                continue
            processes[pid] = {
                "ppid": ppid,
                "uid": uid,
                "cmdline": cmdline,
            }
            children[ppid].append(pid)
        return processes, children


    def descendants_of(root_pid, children):
        seen = set()
        queue = collections.deque(children.get(root_pid, []))
        while queue:
            pid = queue.popleft()
            if pid in seen:
                continue
            seen.add(pid)
            queue.extend(children.get(pid, []))
        return seen


    def set_adj_if_needed(pid, target, reason, cmdline):
        try:
            current = read_adj(pid)
            if current == target:
                return
            write_adj(pid, target)
        except FileNotFoundError:
            log(
                "WARN",
                f"pid={pid} exited before oom_score_adj write for {reason}",
            )
            return
        except PermissionError as e:
            log(
                "WARN",
                "permission denied writing oom_score_adj "
                f"for pid={pid} reason={reason}: {e}",
            )
            return
        except OSError as e:
            log(
                "WARN",
                f"failed writing oom_score_adj for pid={pid} "
                f"reason={reason}: {e}",
            )
            return

        display_cmdline = cmdline[:240] if cmdline else "<empty>"
        log(
            "INFO",
            f"set oom_score_adj={target} pid={pid} reason={reason} "
            f"cmdline={display_cmdline}",
        )


    def pause_state_path():
        state_directory = os.environ.get("STATE_DIRECTORY")
        if not state_directory:
            state_directory = os.path.join(
                os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"),
                "dev-oom-guard",
            )
        os.makedirs(state_directory, mode=0o700, exist_ok=True)
        return os.path.join(state_directory, "paused.json")


    def load_paused():
        try:
            with open(pause_state_path(), encoding="utf-8") as f:
                saved = json.load(f)
        except (FileNotFoundError, json.JSONDecodeError, OSError):
            return {}
        return {
            str(pid): str(starttime)
            for pid, starttime in saved.items()
            if str(pid).isdigit()
        }


    def save_paused():
        path = pause_state_path()
        if not paused:
            try:
                os.unlink(path)
            except FileNotFoundError:
                pass
            return
        temporary = f"{path}.tmp"
        with open(temporary, "w", encoding="utf-8") as f:
            json.dump(paused, f, sort_keys=True)
            f.write("\n")
        os.chmod(temporary, 0o600)
        os.replace(temporary, path)


    def resume_paused():
        changed = False
        for pid_text, starttime in list(paused.items()):
            pid = int(pid_text)
            try:
                if process_starttime(pid) == starttime:
                    os.kill(pid, signal.SIGCONT)
            except (FileNotFoundError, ProcessLookupError):
                pass
            except (PermissionError, OSError) as e:
                log("WARN", f"cannot resume paused pid={pid}: {e}")
            paused.pop(pid_text, None)
            changed = True
        if changed:
            save_paused()


    def pause_descendants(descendant_reasons, protected, processes):
        changed = False
        for pid, agent_name in sorted(descendant_reasons.items()):
            if pid in protected or pid not in processes or str(pid) in paused:
                continue
            try:
                starttime = process_starttime(pid)
                if process_state(pid) in ("T", "t"):
                    continue
                os.kill(pid, signal.SIGSTOP)
                paused[str(pid)] = starttime
                log(
                    "INFO",
                    f"paused pid={pid} reason=agent-descendant:{agent_name} "
                    f"cmdline={processes[pid]['cmdline'][:240]}",
                )
                changed = True
            except (FileNotFoundError, ProcessLookupError):
                continue
            except (PermissionError, OSError) as e:
                log("WARN", f"cannot pause pid={pid}: {e}")
        if changed:
            save_paused()


    def matched_protected(processes, protect_re):
        protected = {}
        for pid, proc in processes.items():
            for name, regex, max_adj, verify_cgroup in protect_re:
                if regex.search(proc["cmdline"]):
                    protected[pid] = (name, max_adj, verify_cgroup)
                    break
        return protected


    def protect_once(protect_uids, protect_re):
        processes, _ = read_process_table()
        protected = matched_protected(processes, protect_re)

        for pid, proc in processes.items():
            if proc["uid"] not in protect_uids:
                continue
            match = protected.get(pid)
            is_guard = "/etc/dev-oom-guard/config.json" in proc["cmdline"]
            if match is None and is_guard:
                match = ("dev-oom-guard", -1000, False)
            if match is None:
                continue
            name, max_adj, _ = match
            try:
                current = read_adj(pid)
            except FileNotFoundError:
                continue
            except OSError as e:
                log(
                    "WARN",
                    f"cannot read oom_score_adj for protected pid={pid} "
                    f"protect={name}: {e}",
                )
                continue
            if current > max_adj:
                set_adj_if_needed(
                    pid,
                    max_adj,
                    f"protect:{name}",
                    proc["cmdline"],
                )


    def scan_once(cfg, agents_re, protect_re):
        processes, children = read_process_table()
        protected = matched_protected(processes, protect_re)

        descendant_reasons = {}
        for pid, proc in processes.items():
            cmdline = proc["cmdline"]
            for name, regex in agents_re:
                if regex.search(cmdline):
                    for descendant in descendants_of(pid, children):
                        descendant_reasons.setdefault(descendant, name)

        for pid, agent_name in sorted(descendant_reasons.items()):
            if pid in protected or pid not in processes:
                continue
            set_adj_if_needed(
                pid,
                cfg["killAdj"],
                f"agent-descendant:{agent_name}",
                processes[pid]["cmdline"],
            )

        for pid, (_name, _max_adj, verify_cgroup) in sorted(protected.items()):
            if pid not in processes:
                continue
            if verify_cgroup and cfg.get("verifyEditorScopes", True):
                verify_editor_cgroup(pid, cfg.get("allowDegraded", False))

        pause_path = cfg.get("pausePath")
        if pause_path and os.path.exists(pause_path):
            pause_descendants(descendant_reasons, protected, processes)
        else:
            resume_paused()


    def handle_signal(signum, _frame):
        global running
        running = False


    def compile_agent_regexes(cfg):
        try:
            agents_re = [
                (a["name"], re.compile(a["cmdlineRegex"]))
                for a in cfg["agents"]
            ]
            protect_re = [
                (
                    p["name"],
                    re.compile(p["cmdlineRegex"]),
                    p.get("maxAdj", 0),
                    p.get("verifyEditorCgroup", False),
                )
                for p in cfg["protect"]
            ]
        except re.error as e:
            log("FATAL", f"invalid cmdlineRegex: {e}")
            sys.exit(2)
        return agents_re, protect_re


    def main():
        protect_mode = len(sys.argv) == 3 and sys.argv[1] == "--protect"
        if not protect_mode and len(sys.argv) != 2:
            log(
                "FATAL",
                "usage: dev-oom-guard [--protect] "
                "/etc/dev-oom-guard/config.json",
            )
            sys.exit(2)

        config_path = sys.argv[2] if protect_mode else sys.argv[1]
        with open(config_path, encoding="utf-8") as f:
            cfg = json.load(f)

        agents_re, protect_re = compile_agent_regexes(cfg)
        signal.signal(signal.SIGTERM, handle_signal)
        signal.signal(signal.SIGINT, handle_signal)

        if protect_mode:
            try:
                protect_uids = {
                    pwd.getpwnam(name).pw_uid for name in cfg["protectUsers"]
                }
            except KeyError as e:
                log("FATAL", f"unknown protect user: {e.args[0]}")
                sys.exit(2)
            log(
                "INFO",
                "dev-oom-protector started; "
                f"protectUsers={cfg['protectUsers']}, "
                f"pollSeconds={cfg['pollSeconds']}",
            )
            while running:
                try:
                    protect_once(protect_uids, protect_re)
                except Exception as e:
                    log("WARN", f"protection scan failed: {e}")
                time.sleep(cfg["pollSeconds"])
            return

        startup_gate(cfg)
        paused.update(load_paused())
        log(
            "INFO",
            "dev-oom-guard started; "
            f"killAdj={cfg['killAdj']}, pollSeconds={cfg['pollSeconds']}, "
            f"pausePath={cfg.get('pausePath')!r}",
        )
        try:
            while running:
                try:
                    scan_once(cfg, agents_re, protect_re)
                except Exception as e:
                    log("WARN", f"scan failed: {e}")
                time.sleep(cfg["pollSeconds"])
        finally:
            resume_paused()


    if __name__ == "__main__":
        main()
  '';
in {
  options.canix-toolbelt.services.devOomGuard = {
    enable = lib.mkEnableOption "dev-oom-guard (per-user OOM kill-priority biaser)";

    pollSeconds = lib.mkOption {
      type = lib.types.ints.positive;
      default = 5;
      description = "Seconds between /proc scans.";
    };

    pausePath = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = ''
        When this marker exists, suspend matched agent descendants until it is
        removed. Paused PID/start-time pairs are resumed on shutdown.
      '';
    };

    killAdj = lib.mkOption {
      type = lib.types.ints.between (-1000) 1000;
      default = 1000;
      description = "oom_score_adj written to every descendant of every matched agent.";
    };

    allowDegraded = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "If true, do not hard-fail on startup gate; bias what we can.";
    };

    verifyEditorScopes = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Validate the user-manager OOM contract at startup and the actual
        cgroup of protect entries that opt into verifyEditorCgroup.
      '';
    };

    protectUsers = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = ''
        Users whose matching processes the root-owned protector may adjust.
        Keeping this explicit prevents process-name matches from affecting
        system services or other local users.
      '';
    };

    agents = lib.mkOption {
      type = lib.types.listOf (lib.types.submodule matcherSubmodule);
      default = [
        {
          name = "claude-code";
          cmdlineRegex = "anthropic\\.claude-code.*native-binary/claude";
        }
        {
          name = "codex";
          cmdlineRegex = "openai\\.chatgpt.*codex app-server";
        }
      ];
      description = "Agent process matchers whose descendants should receive killAdj.";
    };

    protect = lib.mkOption {
      type = lib.types.listOf (lib.types.submodule protectSubmodule);
      default = [
        {
          name = "rust-analyzer";
          cmdlineRegex = "rust-analyzer";
        }
        {
          name = "pyright";
          cmdlineRegex = "pyright-langserver";
        }
        {
          name = "gopls";
          cmdlineRegex = "gopls";
        }
        {
          name = "tsserver";
          cmdlineRegex = "typescript.*tsserver";
        }
        {
          name = "clangd";
          cmdlineRegex = "clangd";
        }
      ];
      description = "Process matchers whose oom_score_adj should be capped each scan.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.user.settings.Manager.DefaultOOMPolicy = lib.mkDefault "continue";

    environment.etc."dev-oom-guard/config.json".source = jsonFormat.generate "dev-oom-guard.json" {
      inherit (cfg) pollSeconds killAdj allowDegraded verifyEditorScopes agents protect protectUsers pausePath;
    };

    systemd.services.dev-oom-protector = lib.mkIf (cfg.protectUsers != []) {
      description = "Apply declarative OOM protection to selected users' processes";
      wantedBy = ["multi-user.target"];
      serviceConfig = {
        ExecStart = "${devOomGuardScript} --protect /etc/dev-oom-guard/config.json";
        Restart = "on-failure";
        RestartSec = "10s";
        OOMScoreAdjust = -1000;
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectHome = true;
        ProtectSystem = "strict";
        RestrictAddressFamilies = ["AF_UNIX"];
      };
    };

    systemd.user.services.dev-oom-guard = {
      description = "Per-user OOM kill-priority biaser for AI-agent descendants";
      wantedBy = ["default.target"];
      serviceConfig = {
        ExecStart = "${devOomGuardScript} /etc/dev-oom-guard/config.json";
        Restart = "on-failure";
        RestartSec = "10s";
        OOMScoreAdjust = -1000;
        StateDirectory = "dev-oom-guard";
        StateDirectoryMode = "0700";
      };
    };
  };
}
