//! Durable, host-agnostic scheduling of idempotent systemd stages.
//!
//! Deployment supplies units and policy; this engine owns locking, checkpoints,
//! retry budgets, interruption, reports, and restoration of quiesced workers.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// One idempotent, checkpoint-aware unit owned by the consumer.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    /// Stable stage identity.
    pub name: String,
    /// Fully qualified systemd service name.
    pub unit: String,
}

/// Immutable policy for a run. Changes cannot reuse an interrupted run's markers.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OperatorConfig {
    /// Operator identity.
    pub name: String,
    /// Consumer-provided executable and argument contract for interrupted runs.
    pub contract_id: String,
    /// Private durable state directory.
    pub state_dir: PathBuf,
    /// Marker requesting reboot recovery.
    pub request_path: PathBuf,
    /// Marker visible to workload admission gates while executing.
    pub sentinel_path: PathBuf,
    /// Ordered stages.
    pub stages: Vec<Stage>,
    /// Maximum failures allowed per stage across interruptions.
    pub max_attempts: usize,
    /// Delay for each retry, in milliseconds, seconds, minutes, or hours.
    pub retry_delays: Vec<String>,
    /// Workers to stop while running; only previously active units are restored.
    pub quiesce_units: Vec<String>,
}

/// Terminal or resumable result; the CLI preserves the established exit codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// All stages completed successfully.
    Succeeded,
    /// At least one stage exhausted its retry budget.
    Failed,
    /// Cancellation signal left durable recovery intent.
    Interrupted,
    /// A cancelled run's stale request was cleaned without executing stages.
    Cancelled,
}

/// Result observed from the service manager, including the unit's own status.
#[derive(Debug)]
pub struct StageResult {
    /// Both systemctl and the unit report success.
    pub success: bool,
    /// systemd Result property.
    pub result: String,
    /// systemd ExecMainStatus property.
    pub exit_status: i32,
}

/// Service-manager boundary, replaceable with a deterministic localhost mock.
pub trait ServiceManager {
    /// Query whether a worker is currently active.
    fn active(&mut self, unit: &str) -> io::Result<bool>;
    /// Stop a worker or interrupted stage and wait for completion.
    fn stop(&mut self, unit: &str) -> io::Result<()>;
    /// Restore a worker previously recorded active.
    fn restore(&mut self, unit: &str) -> io::Result<()>;
    /// Start a stage, wait, and stop it when interrupted.
    fn start(&mut self, unit: &str, cancelled: &AtomicBool) -> io::Result<StageResult>;
}

#[derive(Serialize, Deserialize)]
struct State {
    run_id: String,
    state: String,
    result: String,
    #[serde(default)]
    config: Option<OperatorConfig>,
    #[serde(default)]
    completed: Vec<String>,
    #[serde(default)]
    failures: BTreeMap<String, usize>,
    #[serde(default)]
    restore_workers: Vec<String>,
    #[serde(default)]
    restoring: bool,
    #[serde(default)]
    request_identity: Option<String>,
    #[serde(default)]
    current_unit: String,
    #[serde(default)]
    current_stage: String,
    #[serde(default)]
    current_index: i64,
    stage_count: usize,
    report: PathBuf,
    updated_at: u64,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn timestamp() -> io::Result<Duration> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)
}

fn durable_dir(path: &Path) -> io::Result<()> {
    let mut missing = Vec::new();
    let mut parent = path;
    while !parent.try_exists()? {
        missing.push(parent.to_path_buf());
        parent = parent
            .parent()
            .ok_or_else(|| invalid("directory has no existing ancestor"))?;
    }
    fs::create_dir_all(path)?;
    for directory in missing.iter().rev() {
        File::open(directory)?.sync_all()?;
        File::open(
            directory
                .parent()
                .ok_or_else(|| invalid("directory has no parent"))?,
        )?
        .sync_all()?;
    }
    Ok(())
}

fn atomic_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let mut file = File::create(&tmp)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    File::open(
        path.parent()
            .ok_or_else(|| invalid("state has no parent"))?,
    )?
    .sync_all()
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => File::open(
            path.parent()
                .ok_or_else(|| invalid("marker has no parent"))?,
        )?
        .sync_all(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn marker(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("marker has no parent"))?;
    durable_dir(parent)?;
    File::create(path)?.sync_all()?;
    File::open(parent)?.sync_all()
}

fn request_identity(path: &Path) -> io::Result<Option<String>> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(format!(
            "{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.mtime(),
            metadata.mtime_nsec(),
        ))),
        Ok(_) => Err(invalid("operator request must be a regular file")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn lock(config: &OperatorConfig) -> io::Result<File> {
    durable_dir(&config.state_dir)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(config.state_dir.join("operator.lock"))?;
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive).map_err(
        |error| {
            if error == rustix::io::Errno::WOULDBLOCK {
                io::Error::new(io::ErrorKind::WouldBlock, "operator is already running")
            } else {
                error.into()
            }
        },
    )?;
    Ok(file)
}

fn delay(value: &str) -> io::Result<Duration> {
    for (suffix, multiplier) in [
        ("ms", 1),
        ("min", 60_000),
        ("s", 1_000),
        ("m", 60_000),
        ("h", 3_600_000),
    ] {
        if let Some(number) = value.strip_suffix(suffix) {
            return number
                .parse::<u64>()
                .ok()
                .and_then(|n| n.checked_mul(multiplier))
                .map(Duration::from_millis)
                .ok_or_else(|| invalid("invalid retry delay"));
        }
    }
    Err(invalid("retry delay must use ms, s, min, m, or h"))
}

fn validate(config: &OperatorConfig) -> io::Result<Vec<Duration>> {
    if config.contract_id.is_empty()
        || config.stages.is_empty()
        || config.max_attempts == 0
        || config.retry_delays.len() < config.max_attempts - 1
    {
        return Err(invalid(
            "operator requires stages and a positive retry budget with delays",
        ));
    }
    let mut names = std::collections::BTreeSet::new();
    for stage in &config.stages {
        if stage.name.is_empty() || !names.insert(&stage.name) {
            return Err(invalid("stage identities must be unique and nonempty"));
        }
    }
    for unit in config
        .stages
        .iter()
        .map(|s| &s.unit)
        .chain(&config.quiesce_units)
    {
        if !unit.ends_with(".service")
            || unit.starts_with('-')
            || !unit
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.@:+".contains(&b))
        {
            return Err(invalid("invalid service unit name"));
        }
    }
    config.retry_delays.iter().map(|s| delay(s)).collect()
}

fn save(config: &OperatorConfig, state: &mut State) -> io::Result<()> {
    state.updated_at = timestamp()?.as_secs();
    atomic_json(&config.state_dir.join("state.json"), state)
}

fn event(
    config: &OperatorConfig,
    state: &State,
    name: &str,
    attempt: usize,
    result: &str,
    status: i32,
) -> io::Result<()> {
    let path = config
        .state_dir
        .join("runs")
        .join(&state.run_id)
        .join("events.jsonl");
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(
        &mut file,
        &serde_json::json!({
            "event": name, "stage": state.current_stage, "unit": state.current_unit,
            "attempt": attempt, "result": result, "exit_status": status, "at": timestamp()?.as_secs(),
        }),
    )?;
    file.write_all(b"\n")?;
    file.sync_all()
}

fn wait(duration: Duration, cancelled: &AtomicBool) {
    let end = std::time::Instant::now() + duration;
    while !cancelled.load(Ordering::SeqCst) && std::time::Instant::now() < end {
        std::thread::sleep(
            Duration::from_millis(50).min(end.saturating_duration_since(std::time::Instant::now())),
        );
    }
}

/// Execute or resume an immutable run, keeping the lock through worker recovery.
/// Legacy shell checkpoints are preserved and rejected until explicitly retired.
pub fn run(
    config: &OperatorConfig,
    services: &mut impl ServiceManager,
    cancelled: &AtomicBool,
) -> io::Result<Outcome> {
    run_with_ready(config, services, cancelled, || Ok(()))
}

/// Run with a readiness callback after the durable admission fence is installed
/// and consumer workers are quiesced. A refused contract never announces ready.
pub fn run_with_ready(
    config: &OperatorConfig,
    services: &mut impl ServiceManager,
    cancelled: &AtomicBool,
    ready: impl FnOnce() -> io::Result<()>,
) -> io::Result<Outcome> {
    let delays = validate(config)?;
    let _lock = lock(config)?;
    let requested = request_identity(&config.request_path)?;
    let state_path = config.state_dir.join("state.json");
    let previous: Option<State> = if state_path.exists() {
        Some(serde_json::from_reader(File::open(&state_path)?)?)
    } else {
        None
    };
    let mut state = match previous {
        Some(state)
            if requested.is_some()
                && (state.restoring
                    || matches!(state.state.as_str(), "running" | "interrupted")
                    || (matches!(state.state.as_str(), "succeeded" | "failed" | "cancelled")
                        && (state.request_identity.is_none()
                            || state.request_identity == requested))) =>
        {
            if state.config.as_ref() != Some(config) {
                return Err(invalid(
                    "interrupted operator contract changed (or legacy shell state); preserve the run and reconcile it before starting a new contract",
                ));
            }
            state
        }
        Some(state) if !state.restore_workers.is_empty() || !state.current_unit.is_empty() => {
            return Err(invalid(format!(
                "run {} has unrecovered workers; retain its request and recover the original contract",
                state.run_id
            )));
        }
        _ => {
            let run_id = timestamp()?.as_nanos().to_string();
            let run_dir = config.state_dir.join("runs").join(&run_id);
            durable_dir(&run_dir)?;
            State {
                run_id,
                state: "running".into(),
                result: "running".into(),
                config: Some(config.clone()),
                completed: vec![],
                failures: BTreeMap::new(),
                restore_workers: vec![],
                restoring: false,
                request_identity: None,
                current_unit: String::new(),
                current_stage: String::new(),
                current_index: -1,
                stage_count: config.stages.len(),
                report: run_dir.join("report.json"),
                updated_at: 0,
            }
        }
    };
    let restore_only =
        state.restoring || matches!(state.state.as_str(), "succeeded" | "failed" | "cancelled");
    // Reboot recovery must not rewrite the marker. Its persisted inode/time
    // identity distinguishes leftover terminal intent from a new touch/replacement.
    if requested.is_none() {
        marker(&config.request_path)?;
    }
    state.request_identity = request_identity(&config.request_path)?;
    if !restore_only {
        state.state = "running".into();
        state.result = "running".into();
    }
    save(config, &mut state)?;
    let execution: io::Result<Outcome> = (|| {
        // A prior crash may have left a stage owned by the service manager.
        // Confirm it stopped before notifying readiness or starting more work.
        if !state.current_unit.is_empty() && !state.completed.contains(&state.current_stage) {
            stop_confirmed(services, &state.current_unit)?;
            state.current_unit.clear();
            save(config, &mut state)?;
        }
        // Record restoration intent before stopping workers so abrupt reboot
        // cannot forget which workers this run owns.
        if !restore_only {
            for unit in &config.quiesce_units {
                if !state.restore_workers.contains(unit) && services.active(unit)? {
                    state.restore_workers.push(unit.clone());
                    save(config, &mut state)?;
                }
                if state.restore_workers.contains(unit) {
                    stop_confirmed(services, unit)?;
                }
            }
        }
        marker(&config.sentinel_path)?;
        ready()?;
        if restore_only {
            return match state.result.as_str() {
                "succeeded" => Ok(Outcome::Succeeded),
                "failed" => Ok(Outcome::Failed),
                "interrupted" => Ok(Outcome::Interrupted),
                "cancelled" => Ok(Outcome::Cancelled),
                _ => Err(invalid(
                    "restoration state has no recorded execution result",
                )),
            };
        }
        let mut failed = false;
        for (index, stage) in config.stages.iter().enumerate() {
            if cancelled.load(Ordering::SeqCst) {
                return Ok(Outcome::Interrupted);
            }
            state.current_stage = stage.name.clone();
            state.current_unit = stage.unit.clone();
            state.current_index = index as i64;
            if state.completed.contains(&stage.name) {
                event(config, &state, "skipped", 0, "success", 0)?;
                continue;
            }
            save(config, &mut state)?;
            let mut failures = *state.failures.get(&stage.name).unwrap_or(&0);
            while failures < config.max_attempts {
                if cancelled.load(Ordering::SeqCst) {
                    return Ok(Outcome::Interrupted);
                }
                state.current_unit = stage.unit.clone();
                save(config, &mut state)?;
                let result = services
                    .start(&stage.unit, cancelled)
                    .unwrap_or_else(|error| StageResult {
                        success: false,
                        result: format!("service-manager-error: {error}"),
                        exit_status: -1,
                    });
                if cancelled.load(Ordering::SeqCst) {
                    return Ok(Outcome::Interrupted);
                }
                if result.success {
                    state.completed.push(stage.name.clone());
                    save(config, &mut state)?;
                    event(
                        config,
                        &state,
                        "completed",
                        failures + 1,
                        &result.result,
                        result.exit_status,
                    )?;
                    break;
                }
                failures += 1;
                state.failures.insert(stage.name.clone(), failures);
                save(config, &mut state)?;
                event(
                    config,
                    &state,
                    "attempt_failed",
                    failures,
                    &result.result,
                    result.exit_status,
                )?;
                // Errors from reset-failed, start or result queries consume the
                // same persisted retry budget. A retry requires a stopped unit.
                stop_confirmed(services, &stage.unit)?;
                if failures < config.max_attempts {
                    wait(delays[failures - 1], cancelled);
                }
            }
            if !state.completed.contains(&stage.name) {
                failed = true;
                event(config, &state, "failed", failures, "failed", 1)?;
            }
        }
        Ok(if failed {
            Outcome::Failed
        } else {
            Outcome::Succeeded
        })
    })();
    // A dead systemctl facade is not proof that systemd stopped its unit.
    // Retain both fences and worker ownership if stage termination is uncertain.
    if !state.current_unit.is_empty() && !state.completed.contains(&state.current_stage) {
        if let Err(error) = stop_confirmed(services, &state.current_unit) {
            state.state = "interrupted".into();
            save(config, &mut state)?;
            return Err(error);
        }
    }
    state.current_unit.clear();
    state.restoring = true;
    state.result = match &execution {
        Ok(Outcome::Succeeded) => "succeeded",
        Ok(Outcome::Failed) => "failed",
        Ok(Outcome::Cancelled) => "cancelled",
        _ => "interrupted",
    }
    .into();
    save(config, &mut state)?;
    // Every execution error passes through worker recovery. Failed recovery
    // retains the durable restore list and request instead of claiming success.
    let mut recovery_error = remove(&config.sentinel_path).err();
    for unit in state.restore_workers.clone() {
        match services.restore(&unit) {
            Ok(()) => {
                state.restore_workers.retain(|s| s != &unit);
                if let Err(error) = save(config, &mut state) {
                    recovery_error.get_or_insert(error);
                }
            }
            Err(error) => {
                recovery_error.get_or_insert(error);
            }
        }
    }
    let outcome = match &execution {
        Ok(outcome) if recovery_error.is_none() => *outcome,
        _ => Outcome::Interrupted,
    };
    state.state = match outcome {
        Outcome::Succeeded => "succeeded",
        Outcome::Failed => "failed",
        Outcome::Interrupted => "interrupted",
        Outcome::Cancelled => "cancelled",
    }
    .into();
    state.restoring = recovery_error.is_some();
    if !state.restoring {
        state.result = state.state.clone();
    }
    save(config, &mut state)?;
    let events = fs::read_to_string(
        config
            .state_dir
            .join("runs")
            .join(&state.run_id)
            .join("events.jsonl"),
    )
    .or_else(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            Ok(String::new())
        } else {
            Err(error)
        }
    })?;
    let events = events
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<Result<Vec<_>, _>>()?;
    atomic_json(
        &state.report,
        &serde_json::json!({"run_id": state.run_id, "result": state.result, "events": events}),
    )?;
    if outcome != Outcome::Interrupted {
        remove(&config.request_path)?;
    }
    if let Some(error) = recovery_error {
        return Err(error);
    }
    execution.map(|_| outcome)
}

/// Cancel only after the main service has stopped; never edit state under its lock.
pub fn cancel(config: &OperatorConfig) -> io::Result<()> {
    validate(config)?;
    let _lock = lock(config)?;
    let path = config.state_dir.join("state.json");
    if path.exists() {
        let mut state: State = serde_json::from_reader(File::open(&path)?)?;
        if !state.restore_workers.is_empty()
            || !state.current_unit.is_empty()
            || config.sentinel_path.exists()
        {
            return Err(invalid(
                "cannot cancel with unrecovered workers or an unconfirmed stage",
            ));
        }
        state.state = "cancelled".into();
        state.result = "cancelled".into();
        save(config, &mut state)?;
    }
    remove(&config.request_path)
}

/// systemctl adapter. The executable is deployment-provided, never a shell command.
pub struct Systemd {
    /// Absolute path to systemctl.
    pub systemctl: PathBuf,
}

fn stop_confirmed(services: &mut impl ServiceManager, unit: &str) -> io::Result<()> {
    services.stop(unit)?;
    if services.active(unit)? {
        return Err(io::Error::other(format!(
            "stage {unit} remains active after stop"
        )));
    }
    Ok(())
}

impl Systemd {
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.systemctl);
        cmd.args(args).stdin(Stdio::null());
        cmd
    }
    fn checked(&self, args: &[&str]) -> io::Result<()> {
        let status = self.command(args).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("systemctl {args:?}: {status}")))
        }
    }
    fn property(&self, unit: &str, property: &str) -> io::Result<String> {
        let result = self
            .command(&["show", "-p", property, "--value", unit])
            .output()?;
        if !result.status.success() {
            return Err(io::Error::other("cannot read service result"));
        }
        Ok(String::from_utf8(result.stdout)
            .map_err(io::Error::other)?
            .trim()
            .to_string())
    }
}

impl ServiceManager for Systemd {
    fn active(&mut self, unit: &str) -> io::Result<bool> {
        let state = self.property(unit, "ActiveState")?;
        Ok(matches!(
            state.as_str(),
            "active" | "activating" | "reloading" | "deactivating"
        ))
    }
    fn stop(&mut self, unit: &str) -> io::Result<()> {
        self.checked(&["stop", unit])
    }
    fn restore(&mut self, unit: &str) -> io::Result<()> {
        self.checked(&["start", unit])
    }
    fn start(&mut self, unit: &str, cancelled: &AtomicBool) -> io::Result<StageResult> {
        // Validate the installed unit without assuming show keeps it loaded:
        // systemd may collect inactive units before the next command.
        let load_state = self.property(unit, "LoadState")?;
        if load_state != "loaded" {
            return Err(io::Error::other(format!(
                "cannot load systemd stage {unit}: {load_state}"
            )));
        }
        // Failed units stay loaded; cold units need no reset and start loads them.
        // Reset/query errors still consume the same persisted retry budget.
        if self.property(unit, "ActiveState")? == "failed" {
            self.checked(&["reset-failed", unit])?;
        }
        let mut child = self.command(&["start", "--wait", unit]).spawn()?;
        let status = loop {
            if cancelled.load(Ordering::SeqCst) {
                let stop = self.stop(unit);
                let _ = child.kill();
                let _ = child.wait();
                stop?;
                return Ok(StageResult {
                    success: false,
                    result: "interrupted".into(),
                    exit_status: 143,
                });
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let result = self.property(unit, "Result")?;
        let exit_status = self
            .property(unit, "ExecMainStatus")?
            .parse()
            .map_err(io::Error::other)?;
        Ok(StageResult {
            success: status.success() && result == "success",
            result,
            exit_status,
        })
    }
}
