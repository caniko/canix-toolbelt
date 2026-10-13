use super::*;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Exact source/input identity shared by required executions.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Composition {
    /// Version of this identity format.
    pub schema_version: u32,
    /// Named source inventory identities.
    pub sources: ArtifactIndex,
    /// Named input-file identities (locks, fixtures, policies, environment configuration).
    pub inputs: ArtifactIndex,
}

/// An explicit execution request, independent of the caller's workspace defaults.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunRequest {
    /// Command working directory.
    pub cwd: PathBuf,
    /// Every source root participating in qualification.
    pub sources: BTreeMap<String, InventorySpec>,
    /// Extra inputs hashed before and after execution.
    pub inputs: BTreeMap<String, PathBuf>,
    /// Argument vector, passed without shell interpolation.
    pub command: Vec<String>,
    /// Explicit overrides, recorded verbatim. Do not put secrets here.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// Explicit tool paths whose bytes and --version output are retained.
    #[serde(default)]
    pub tools: BTreeMap<String, PathBuf>,
    /// Required finite deadline in seconds.
    pub timeout_seconds: u64,
}

/// Execution state. Only Passed can satisfy a required gate.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RunStatus {
    /// No completed receipt exists yet.
    Pending,
    /// Exit zero, before/after identities equal.
    Passed,
    /// Nonzero exit, spawn failure, or evidence-capture failure.
    Failed,
    /// Cancellation requested; any child was stopped and reaped.
    Cancelled,
    /// Required execution deadline exceeded.
    TimedOut,
    /// Selected source, tools or inputs changed during execution.
    SourceChanged,
}

/// Retained identity and version of an execution tool.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolEvidence {
    /// Canonical executable path.
    pub path: PathBuf,
    /// Executable-byte identity.
    pub sha256: String,
    /// Successful --version output captured before the main command.
    pub version: String,
}

/// A completed source-bound execution, not a claim of overall acceptance.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    /// Receipt format version, currently 1.
    pub schema_version: u32,
    /// Effective execution disposition.
    pub status: RunStatus,
    /// Child exit code; absent for signals or spawn failures.
    pub execution_exit: Option<i32>,
    /// Before-execution source/input identity.
    pub composition: Composition,
    /// After-execution identity; absent when capture fails.
    pub after: Option<Composition>,
    /// Selected files before execution, retained for portable verification.
    pub inventories: BTreeMap<String, Inventory>,
    /// Request-file SHA-256, binding argv, cwd, timeout and explicit environment.
    pub request_sha256: String,
    /// stdout.log SHA-256.
    pub stdout_sha256: String,
    /// stderr.log SHA-256.
    pub stderr_sha256: String,
    /// Tool path, binary and version evidence.
    pub tools: BTreeMap<String, ToolEvidence>,
    /// Canonical program actually executed and its byte identity, if resolution succeeded.
    pub executable: Option<ProgramIdentity>,
    /// Relevant inherited build environment; credentials are intentionally excluded.
    pub environment: BTreeMap<String, String>,
    /// UNIX start time in milliseconds.
    pub started_unix_ms: u128,
    /// Monotonic elapsed time in milliseconds.
    pub elapsed_ms: u128,
    /// Execution/capture failure details, when present.
    pub error: Option<String>,
}

/// Byte-bound identity of the main command, resolved before spawning it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramIdentity {
    /// Absolute canonical executable path.
    pub path: PathBuf,
    /// Executable SHA-256.
    pub sha256: String,
}

fn program(request: &RunRequest) -> io::Result<ProgramIdentity> {
    let name = &request.command[0];
    let path = if Path::new(name).components().count() > 1 || Path::new(name).is_absolute() {
        request.cwd.join(name).canonicalize()?
    } else {
        let search = request
            .environment
            .get("PATH")
            .cloned()
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default();
        std::env::split_paths(&search)
            .map(|part| request.cwd.join(part).join(name))
            .find(|path| path.is_file())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("program not found: {name}"),
                )
            })?
            .canonicalize()?
    };
    Ok(ProgramIdentity {
        sha256: sha256_file(&path)?,
        path,
    })
}

fn build_environment(request: &RunRequest) -> BTreeMap<String, String> {
    [
        "PATH",
        "CARGO_HOME",
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_JOBS",
        "RUSTC",
        "RUSTDOC",
        "RUSTC_WRAPPER",
        "RUSTC_BOOTSTRAP",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTDOCFLAGS",
        "CI",
        "TRYBUILD",
        "MACROTEST",
        "JAVA_HOME",
        "LD_LIBRARY_PATH",
        "TMPDIR",
    ]
    .into_iter()
    .filter_map(|name| {
        request
            .environment
            .get(name)
            .cloned()
            .or_else(|| std::env::var(name).ok())
            .map(|value| (name.into(), value))
    })
    .collect()
}

fn capture(request: &RunRequest) -> io::Result<(Composition, BTreeMap<String, Inventory>)> {
    if request.sources.is_empty() {
        return Err(invalid(
            "qualification requires a nonempty source selection",
        ));
    }
    let inventories = request
        .sources
        .iter()
        .map(|(name, spec)| Ok((name.clone(), inventory(spec)?)))
        .collect::<io::Result<BTreeMap<_, _>>>()?;
    let sources = inventories
        .iter()
        .map(|(name, value)| Ok((name.clone(), fingerprint(value)?)))
        .collect::<io::Result<_>>()?;
    let inputs = request
        .inputs
        .iter()
        .map(|(name, path)| Ok((name.clone(), sha256_file(path)?)))
        .collect::<io::Result<_>>()?;
    Ok((
        Composition {
            schema_version: 1,
            sources,
            inputs,
        },
        inventories,
    ))
}

fn stop(child: &mut Child) -> io::Result<()> {
    #[cfg(unix)]
    {
        if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
            // The child owns a fresh process group; stop descendants, too.
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
    }
    let _ = child.kill();
    child.wait().map(|_| ())
}

fn wait(
    child: &mut Child,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> io::Result<(RunStatus, Option<i32>)> {
    loop {
        if cancelled.load(Ordering::Relaxed) {
            stop(child)?;
            return Ok((RunStatus::Cancelled, None));
        }
        if Instant::now() >= deadline {
            stop(child)?;
            return Ok((RunStatus::TimedOut, None));
        }
        if let Some(status) = child.try_wait()? {
            #[cfg(unix)]
            if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
                if rustix::process::test_kill_process_group(pid).is_ok() {
                    stop(child)?;
                    return Ok((RunStatus::Failed, status.code()));
                }
            }
            return Ok((
                if status.success() {
                    RunStatus::Passed
                } else {
                    RunStatus::Failed
                },
                status.code(),
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn bounded_output(
    program: &Path,
    args: &[&str],
    request: &RunRequest,
    cancelled: &AtomicBool,
    directory: &Path,
    name: &str,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    let path = directory.join(name);
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(&request.cwd)
        .envs(&request.environment)
        .stdin(Stdio::null())
        .stdout(File::create(&path)?)
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    let result = wait(&mut child, deadline, cancelled);
    if result.is_err() {
        let _ = stop(&mut child);
    }
    if result? != (RunStatus::Passed, Some(0)) {
        return Err(invalid("tool --version did not complete successfully"));
    }
    fs::read(path)
}

/// Execute once into a newly created evidence directory. Existing evidence is never reused.
/// Inherited environment is supplied by the approved caller; explicit overrides are retained.
/// The state directory must be outside all selected source directories.
pub fn execute(
    request: &RunRequest,
    directory: &Path,
    cancelled: &AtomicBool,
) -> io::Result<Receipt> {
    if request.command.is_empty()
        || request.command[0].is_empty()
        || request.timeout_seconds == 0
        || request.timeout_seconds > 86400
    {
        return Err(invalid(
            "command and a deadline between 1 and 86400 seconds are required",
        ));
    }
    let cwd = request.cwd.canonicalize()?;
    let parent = directory
        .parent()
        .ok_or_else(|| invalid("evidence directory has no parent"))?
        .canonicalize()?;
    let destination = parent.join(
        directory
            .file_name()
            .ok_or_else(|| invalid("evidence directory has no name"))?,
    );
    // Explicit file selections allow a state sibling in cwd; recursive selections cannot include it.
    for spec in request.sources.values() {
        let root = spec.root.canonicalize()?;
        for path in &spec.paths {
            let source = contained(&root, path)?;
            if source.is_dir() && destination.starts_with(&source) {
                return Err(invalid("evidence directory is inside selected source"));
            }
        }
    }
    let (composition, inventories) = capture(request)?;
    fs::create_dir(&destination)?;
    write_json(&destination.join("request.json"), request)?;
    write_json(&destination.join("pending.json"), &RunStatus::Pending)?;
    let started = Instant::now();
    let mut receipt = Receipt {
        schema_version: 1,
        status: RunStatus::Pending,
        execution_exit: None,
        composition,
        after: None,
        inventories,
        request_sha256: sha256_file(&destination.join("request.json"))?,
        stdout_sha256: String::new(),
        stderr_sha256: String::new(),
        tools: BTreeMap::new(),
        executable: None,
        environment: build_environment(request),
        started_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| invalid(e.to_string()))?
            .as_millis(),
        elapsed_ms: 0,
        error: None,
    };
    // Create logs even for cancellation or spawn failures, for a complete failed receipt.
    let out = File::create(destination.join("stdout.log"))?;
    let err = File::create(destination.join("stderr.log"))?;
    let outcome = (|| -> io::Result<(RunStatus, Option<i32>)> {
        if cancelled.load(Ordering::Relaxed) {
            return Ok((RunStatus::Cancelled, None));
        }
        for (name, path) in &request.tools {
            relative(Path::new(name))?;
            if Path::new(name).components().count() != 1 {
                return Err(invalid("tool labels must be single path components"));
            }
            let path = path.canonicalize()?;
            let sha256 = sha256_file(&path)?;
            let output = bounded_output(
                &path,
                &["--version"],
                request,
                cancelled,
                &destination,
                &format!("tool-{name}.log"),
                started + Duration::from_secs(request.timeout_seconds),
            )?;
            let version = String::from_utf8(output).map_err(|e| invalid(e.to_string()))?;
            receipt.tools.insert(
                name.clone(),
                ToolEvidence {
                    path,
                    sha256,
                    version,
                },
            );
        }
        let executable = program(request)?;
        let mut command = Command::new(&executable.path);
        receipt.executable = Some(executable);
        command
            .args(&request.command[1..])
            .current_dir(&cwd)
            .envs(&request.environment)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn()?;
        let outcome = wait(
            &mut child,
            started + Duration::from_secs(request.timeout_seconds),
            cancelled,
        );
        if outcome.is_err() {
            let _ = stop(&mut child);
        }
        outcome
    })();
    match outcome {
        Ok((status, exit)) => {
            receipt.status = status;
            receipt.execution_exit = exit;
            if status == RunStatus::Failed && exit == Some(0) {
                receipt.error = Some("producer exited while descendants were still running".into());
            }
        }
        Err(error) => {
            receipt.status = RunStatus::Failed;
            receipt.error = Some(error.to_string());
        }
    }
    if cancelled.load(Ordering::Relaxed) {
        receipt.status = RunStatus::Cancelled;
    } else if receipt.status == RunStatus::Failed
        && started.elapsed() >= Duration::from_secs(request.timeout_seconds)
    {
        receipt.status = RunStatus::TimedOut;
    }
    match capture(request) {
        Ok((after, _)) => {
            if after != receipt.composition {
                receipt.status = RunStatus::SourceChanged;
            }
            receipt.after = Some(after);
        }
        Err(error) => {
            receipt.status = RunStatus::SourceChanged;
            receipt.error = Some(error.to_string());
        }
    }
    for tool in receipt.tools.values() {
        if sha256_file(&tool.path).ok().as_ref() != Some(&tool.sha256) {
            receipt.status = RunStatus::SourceChanged;
        }
    }
    if let Some(executable) = &receipt.executable {
        if sha256_file(&executable.path).ok().as_ref() != Some(&executable.sha256) {
            receipt.status = RunStatus::SourceChanged;
        }
    }
    receipt.elapsed_ms = started.elapsed().as_millis();
    receipt.stdout_sha256 = sha256_file(&destination.join("stdout.log"))?;
    receipt.stderr_sha256 = sha256_file(&destination.join("stderr.log"))?;
    write_json(&destination.join("receipt.json"), &receipt)?;
    Ok(receipt)
}

/// Verify completed receipt integrity and, when supplied, the required immutable composition.
/// This is portable evidence verification; it does not rerun commands or trust signatures.
pub fn verify_run(directory: &Path, expected: Option<&Composition>) -> io::Result<Receipt> {
    let receipt: Receipt = read_json(&directory.join("receipt.json"))?;
    if receipt.schema_version != 1
        || receipt.composition.schema_version != 1
        || receipt.status != RunStatus::Passed
        || receipt.execution_exit != Some(0)
        || receipt.after.as_ref() != Some(&receipt.composition)
        || receipt.error.is_some()
        || receipt.composition.sources.is_empty()
        || receipt.executable.is_none()
        || expected.is_some_and(|expected| expected != &receipt.composition)
    {
        return Err(invalid(
            "receipt is incomplete, failed, changed, or belongs to another composition",
        ));
    }
    for (name, inventory) in &receipt.inventories {
        if inventory.schema_version != 1
            || inventory.files.is_empty()
            || receipt.composition.sources.get(name) != Some(&fingerprint(inventory)?)
        {
            return Err(invalid("receipt source inventory identity differs"));
        }
    }
    if receipt.inventories.len() != receipt.composition.sources.len() {
        return Err(invalid("missing source inventories"));
    }
    for (name, hash) in [
        ("request.json", &receipt.request_sha256),
        ("stdout.log", &receipt.stdout_sha256),
        ("stderr.log", &receipt.stderr_sha256),
    ] {
        if sha256_file(&directory.join(name))? != *hash {
            return Err(invalid(format!("receipt artifact changed: {name}")));
        }
    }
    Ok(receipt)
}
