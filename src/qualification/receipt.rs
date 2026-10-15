use super::*;
use std::process::{Child, Command};
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
    #[serde(deserialize_with = "unique_map")]
    pub sources: BTreeMap<String, InventorySpec>,
    /// Extra inputs hashed before and after execution.
    #[serde(deserialize_with = "unique_map")]
    pub inputs: BTreeMap<String, PathBuf>,
    /// Argument vector, passed without shell interpolation.
    pub command: Vec<String>,
    /// Optional `argv[0]` override. Retained executions set this to the requested
    /// command name while `command[0]` records the canonical executable selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_zero: Option<String>,
    /// Explicit overrides, recorded verbatim. Do not put secrets here.
    #[serde(default, deserialize_with = "unique_map")]
    pub environment: BTreeMap<String, String>,
    /// Explicit tool paths whose bytes and --version output are retained.
    #[serde(default, deserialize_with = "unique_map")]
    pub tools: BTreeMap<String, PathBuf>,
    /// Optional tool `argv[0]` overrides; retained requests bind every effective alias.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "unique_map"
    )]
    pub tool_argument_zeros: BTreeMap<String, String>,
    /// Explicit read-only runtime paths, each also selected by `sources` or `inputs`
    /// so its contents participate in before/after composition capture.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_paths: Vec<PathBuf>,
    /// Required finite deadline in seconds.
    pub timeout_seconds: u64,
    /// Explicit namespace launcher and native supervisor. Required for execution.
    #[serde(default)]
    pub containment: Option<ContainmentPrograms>,
}

/// Caller-selected Linux containment tools; all binaries are byte-bound in receipts.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContainmentPrograms {
    /// util-linux unshare supporting --forward-signals and --kill-child.
    pub unshare: PathBuf,
    /// Packaged canix-toolbelt-qualification-executor binary.
    pub executor: PathBuf,
    /// util-linux setpriv used to drop every producer capability and privilege escalation.
    pub setpriv: PathBuf,
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
    /// Receipt format version, currently 5. Earlier versions exposed host files and sockets.
    pub schema_version: u32,
    /// Effective execution disposition.
    pub status: RunStatus,
    /// Child exit code; absent for signals or spawn failures.
    pub execution_exit: Option<i32>,
    /// Original producer termination signal, distinct from an ordinary exit code.
    pub execution_signal: Option<i32>,
    /// Native supervisor report identity; present after confirmed leader completion.
    pub execution_report_sha256: Option<String>,
    /// Before-execution source/input identity.
    pub composition: Composition,
    /// After-execution identity; absent when capture fails.
    pub after: Option<Composition>,
    /// Selected files before execution, retained for portable verification.
    #[serde(deserialize_with = "unique_map")]
    pub inventories: BTreeMap<String, Inventory>,
    /// Request-file SHA-256, binding argv, cwd, timeout and explicit environment.
    pub request_sha256: String,
    /// stdout.log SHA-256.
    pub stdout_sha256: String,
    /// stderr.log SHA-256.
    pub stderr_sha256: String,
    /// Tool path, binary and version evidence.
    #[serde(deserialize_with = "unique_map")]
    pub tools: BTreeMap<String, ToolEvidence>,
    /// Canonical program actually executed and its byte identity, if resolution succeeded.
    pub executable: Option<ProgramIdentity>,
    /// Namespace launcher and native supervisor identities, when selected.
    #[serde(default, deserialize_with = "unique_map")]
    pub containment: BTreeMap<String, ProgramIdentity>,
    /// Complete environment actually supplied after clearing inherited variables.
    #[serde(deserialize_with = "unique_map")]
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

fn program(
    request: &RunRequest,
    environment: &BTreeMap<String, String>,
    check: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<ProgramIdentity> {
    let name = &request.command[0];
    let path = if Path::new(name).components().count() > 1 || Path::new(name).is_absolute() {
        request.cwd.join(name).canonicalize()?
    } else {
        let search = environment.get("PATH").cloned().unwrap_or_default();
        std::env::split_paths(&search)
            .map(|part| request.cwd.join(part).join(name))
            .find(|path| is_executable(path))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("program not found: {name}"),
                )
            })?
            .canonicalize()?
    };
    Ok(ProgramIdentity {
        sha256: sha256_file_checked(&path, check)?,
        path,
    })
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            && rustix::fs::access(path, rustix::fs::Access::EXEC_OK).is_ok()
    }
    #[cfg(not(unix))]
    path.is_file()
}

fn build_environment(request: &RunRequest) -> BTreeMap<String, String> {
    let mut environment: BTreeMap<_, _> = [
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
        "HOME",
        "USER",
        "LOGNAME",
        "LANG",
        "LC_ALL",
        "TZ",
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
    .collect();
    environment.extend(request.environment.clone());
    environment
}

fn capture(
    request: &RunRequest,
    check: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<(Composition, BTreeMap<String, Inventory>)> {
    if request.sources.is_empty() {
        return Err(invalid(
            "qualification requires a nonempty source selection",
        ));
    }
    let inventories = request
        .sources
        .iter()
        .map(|(name, spec)| Ok((name.clone(), inventory_checked(spec, check)?)))
        .collect::<io::Result<BTreeMap<_, _>>>()?;
    let sources = inventories
        .iter()
        .map(|(name, value)| Ok((name.clone(), fingerprint(value)?)))
        .collect::<io::Result<_>>()?;
    let inputs = request
        .inputs
        .iter()
        .map(|(name, path)| Ok((name.clone(), sha256_file_checked(path, check)?)))
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

// Purely lexical after execution canonicalizes selections; portable verification
// uses the identical rule without consulting the original producer filesystem.
fn validate_runtime_selection(request: &RunRequest) -> io::Result<()> {
    for runtime in &request.runtime_paths {
        if !runtime.is_absolute()
            || runtime == Path::new("/")
            || runtime
                .components()
                .any(|part| matches!(part, Component::ParentDir))
            || !(request.inputs.values().any(|path| path == runtime)
                || request.sources.values().any(|spec| {
                    spec.root.is_absolute()
                        && spec
                            .paths
                            .iter()
                            .any(|path| spec.root.join(path) == *runtime)
                }))
        {
            return Err(invalid(
                "runtime path requires an identical source or input selection for composition binding",
            ));
        }
    }
    Ok(())
}

fn stop(child: &mut Child) -> io::Result<()> {
    let cleanup_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        #[cfg(target_os = "linux")]
        if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
            // Retry until init has installed its signal handler. unshare forwards
            // TERM, then reaps init only after the kernel drains its namespace.
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::CONT);
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
        }
        if Instant::now() >= cleanup_deadline {
            // No final capture or receipt may follow unconfirmed cleanup.
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "could not confirm PID namespace cleanup",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
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

fn containment(
    request: &RunRequest,
    check: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<BTreeMap<String, ProgramIdentity>> {
    if !cfg!(target_os = "linux") {
        return Err(invalid(
            "qualification execution requires Linux PID namespace containment",
        ));
    }
    let tools = request
        .containment
        .as_ref()
        .ok_or_else(|| invalid("explicit containment tools are required"))?;
    [
        ("unshare", &tools.unshare),
        ("executor", &tools.executor),
        ("setpriv", &tools.setpriv),
    ]
    .into_iter()
    .map(|(name, path)| {
        let path = path.canonicalize()?;
        if !is_executable(&path) {
            return Err(invalid("containment tool is not executable"));
        }
        Ok((
            name.into(),
            ProgramIdentity {
                sha256: sha256_file_checked(&path, check)?,
                path,
            },
        ))
    })
    .collect()
}

fn contained_command(
    tools: &BTreeMap<String, ProgramIdentity>,
    executable: &Path,
    argument_zero: &std::ffi::OsStr,
    work: &Path,
    boundary: &Path,
    max_log_bytes: u64,
) -> Command {
    let mut command = Command::new(&tools["unshare"].path);
    command
        .args([
            "--user",
            "--map-root-user",
            "--pid",
            "--net",
            "--ipc",
            "--fork",
            "--mount-proc",
            "--kill-child=KILL",
            "--forward-signals",
            "--",
        ])
        .arg(&tools["executor"].path)
        .arg(work)
        .arg(boundary)
        .arg(max_log_bytes.to_string())
        .arg(&tools["setpriv"].path)
        .arg(executable)
        .arg(argument_zero);
    command
}

fn tool_version(
    tool: (&Path, &str),
    request: &RunRequest,
    cancelled: &AtomicBool,
    directory: &Path,
    name: &str,
    deadline: Instant,
    receipt: &Receipt,
) -> io::Result<Vec<u8>> {
    let (program, argument_zero) = tool;
    let path = directory.join(name);
    let stderr_path = directory.join(format!("{name}.stderr"));
    let mut command = contained_command(
        &receipt.containment,
        program,
        std::ffi::OsStr::new(argument_zero),
        &directory.join("work"),
        &directory.join("boundary.json"),
        1024 * 1024,
    );
    command
        .arg("--version")
        .current_dir(&request.cwd)
        .env_clear()
        .env("LANG", "C")
        .stdin(
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join(format!("{name}.execution.json")))?,
        )
        .stdout(File::create(&path)?)
        .stderr(File::create(&stderr_path)?);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    let result = wait(&mut child, deadline, cancelled);
    if result.is_err() {
        stop(&mut child)?;
    }
    if result? != (RunStatus::Passed, Some(0)) {
        return Err(invalid("tool --version did not complete successfully"));
    }
    for stream in [&path, &stderr_path] {
        if fs::metadata(stream)?.len() > 1024 * 1024 {
            return Err(invalid("tool version stream exceeds 1 MiB"));
        }
    }
    let mut output = fs::read(path)?;
    output.extend_from_slice(&fs::read(stderr_path)?);
    Ok(output)
}

/// Execute once into a newly created evidence directory. Existing evidence is never reused.
/// Inherited environment is cleared; retained allowlisted settings and explicit overrides are supplied.
/// The state directory must be outside all selected source directories.
pub fn execute(
    request: &RunRequest,
    directory: &Path,
    cancelled: &AtomicBool,
) -> io::Result<Receipt> {
    let started = Instant::now();
    let started_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| invalid(e.to_string()))?
        .as_millis();
    if request.command.is_empty()
        || request.command[0].is_empty()
        || request.argument_zero.as_ref().is_some_and(String::is_empty)
        || request.timeout_seconds == 0
        || request.timeout_seconds > 86400
    {
        return Err(invalid(
            "command and a deadline between 1 and 86400 seconds are required",
        ));
    }
    let deadline = started + Duration::from_secs(request.timeout_seconds);
    let mut check = || {
        if cancelled.load(Ordering::Relaxed) {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "evidence capture cancelled",
            ))
        } else if Instant::now() >= deadline {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "evidence capture deadline exceeded",
            ))
        } else {
            Ok(())
        }
    };
    let containment = containment(request, &mut check)?;
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
    let (composition, inventories) = capture(request, &mut check)?;
    // Retain effective canonical selections so portable verification can compare
    // the request with receipt identities without consulting the original host.
    let mut effective_request = request.clone();
    effective_request.argument_zero = Some(
        request
            .argument_zero
            .clone()
            .unwrap_or_else(|| request.command[0].clone()),
    );
    effective_request.cwd = cwd.clone();
    for spec in effective_request.sources.values_mut() {
        spec.root = spec.root.canonicalize()?;
    }
    for spec in request.sources.values() {
        let root = spec.root.canonicalize()?;
        for path in &spec.paths {
            relative(path)?;
            let mut selected = root.clone();
            for part in path.components() {
                if let std::path::Component::Normal(name) = part {
                    selected.push(name);
                    if fs::symlink_metadata(&selected)?.file_type().is_symlink() {
                        return Err(invalid(
                            "selected source mount traverses a symlink; select its containing directory to preserve link semantics",
                        ));
                    }
                }
            }
        }
    }
    effective_request.inputs = request
        .inputs
        .iter()
        .map(|(name, path)| Ok((name.clone(), path.canonicalize()?)))
        .collect::<io::Result<_>>()?;
    effective_request.runtime_paths = request
        .runtime_paths
        .iter()
        .map(|path| {
            if !path.is_absolute() {
                return Err(invalid("runtime mounts must be absolute paths"));
            }
            path.canonicalize()
        })
        .collect::<io::Result<_>>()?;
    validate_runtime_selection(&effective_request)?;
    if request
        .tool_argument_zeros
        .keys()
        .any(|name| !request.tools.contains_key(name))
    {
        return Err(invalid("tool argument zero has no selected tool"));
    }
    effective_request.tool_argument_zeros = request
        .tools
        .iter()
        .map(|(name, path)| {
            let alias = request
                .tool_argument_zeros
                .get(name)
                .cloned()
                .or_else(|| path.to_str().map(str::to_owned))
                .filter(|value| !value.is_empty())
                .ok_or_else(|| invalid("invalid tool argument zero"))?;
            Ok((name.clone(), alias))
        })
        .collect::<io::Result<_>>()?;
    effective_request.tools = request
        .tools
        .iter()
        .map(|(name, path)| Ok((name.clone(), path.canonicalize()?)))
        .collect::<io::Result<_>>()?;
    effective_request.containment = Some(ContainmentPrograms {
        unshare: containment["unshare"].path.clone(),
        executor: containment["executor"].path.clone(),
        setpriv: containment["setpriv"].path.clone(),
    });
    let work = destination.join("work");
    // Only a fresh, source-disjoint work tree is writable in the producer's
    // namespace. Inherited open log descriptors remain writable as well.
    for spec in request.sources.values() {
        let root = spec.root.canonicalize()?;
        for path in &spec.paths {
            let selected = contained(&root, path)?;
            if selected.is_dir() && work.starts_with(selected) {
                return Err(invalid("writable work tree overlaps selected source"));
            }
        }
    }
    let mut environment = build_environment(request);
    environment.insert(
        "QUALIFICATION_WORK_DIR".into(),
        work.to_str()
            .ok_or_else(|| invalid("non-UTF-8 work path"))?
            .into(),
    );
    environment.insert(
        "TMPDIR".into(),
        work.to_str()
            .ok_or_else(|| invalid("non-UTF-8 work path"))?
            .into(),
    );
    environment.insert(
        "HOME".into(),
        work.join("home")
            .to_str()
            .ok_or_else(|| invalid("non-UTF-8 home path"))?
            .into(),
    );
    if !request.environment.contains_key("CARGO_HOME") {
        environment.insert(
            "CARGO_HOME".into(),
            work.join("cargo-home")
                .to_str()
                .ok_or_else(|| invalid("non-UTF-8 Cargo home path"))?
                .into(),
        );
    }
    if !request.environment.contains_key("CARGO_TARGET_DIR") {
        environment.insert(
            "CARGO_TARGET_DIR".into(),
            work.join("target")
                .to_str()
                .ok_or_else(|| invalid("non-UTF-8 target path"))?
                .into(),
        );
    }
    effective_request.environment = environment.clone();
    // Resolve before probes so the retained request binds the path selected from
    // the effective PATH. Byte checks afterward detect changes during execution.
    let executable = program(request, &environment, &mut check);
    if let Ok(executable) = &executable {
        effective_request.command[0] = executable
            .path
            .to_str()
            .ok_or_else(|| invalid("non-UTF-8 executable path"))?
            .to_owned();
    }
    let request = &effective_request;
    fs::create_dir(&destination)?;
    fs::create_dir(&work)?;
    fs::create_dir(work.join("home"))?;
    fs::create_dir(work.join("cargo-home"))?;
    let sandbox = destination.join("sandbox");
    fs::create_dir(&sandbox)?;
    // Public system runtime trees supply dynamic loaders and standard tools;
    // neither the host home, /etc nor host /run is recursively exposed.
    let mut read_only_paths: Vec<PathBuf> = [
        "/nix/store",
        "/bin",
        "/usr/bin",
        "/usr/lib",
        "/usr/lib64",
        "/lib",
        "/lib64",
        "/run/current-system/sw",
        "/dev/null",
        "/dev/zero",
        "/dev/random",
        "/dev/urandom",
    ]
    .into_iter()
    .map(PathBuf::from)
    .filter(|path| path.exists())
    .collect();
    for spec in request.sources.values() {
        let root = spec.root.canonicalize()?;
        for path in &spec.paths {
            read_only_paths.push(root.join(path));
        }
    }
    read_only_paths.extend(request.inputs.values().cloned());
    read_only_paths.extend(request.tools.values().cloned());
    read_only_paths.extend(containment.values().map(|program| program.path.clone()));
    if let Ok(program) = &executable {
        read_only_paths.push(program.path.clone());
    }
    read_only_paths.extend(request.runtime_paths.iter().cloned());
    if read_only_paths
        .iter()
        .any(|path| !path.is_absolute() || path == Path::new("/"))
    {
        return Err(invalid(
            "runtime mounts must be absolute paths below the host root",
        ));
    }
    write_json(
        &destination.join("boundary.json"),
        &super::boundary::Boundary {
            read_only_paths,
            environment: environment.clone(),
            sandbox,
        },
    )?;
    write_json(&destination.join("request.json"), request)?;
    write_json(&destination.join("pending.json"), &RunStatus::Pending)?;
    let mut receipt = Receipt {
        schema_version: 5,
        status: RunStatus::Pending,
        execution_exit: None,
        execution_signal: None,
        execution_report_sha256: None,
        composition,
        after: None,
        inventories,
        request_sha256: sha256_file(&destination.join("request.json"))?,
        stdout_sha256: String::new(),
        stderr_sha256: String::new(),
        tools: BTreeMap::new(),
        executable: executable.as_ref().ok().cloned(),
        containment,
        environment,
        started_unix_ms,
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
            let sha256 = sha256_file_checked(&path, &mut check)?;
            let output = tool_version(
                (&path, &request.tool_argument_zeros[name]),
                request,
                cancelled,
                &destination,
                &format!("tool-{name}.log"),
                started + Duration::from_secs(request.timeout_seconds),
                &receipt,
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
        let executable = executable?;
        let mut command = contained_command(
            &receipt.containment,
            &executable.path,
            std::ffi::OsStr::new(
                request
                    .argument_zero
                    .as_deref()
                    .unwrap_or(&request.command[0]),
            ),
            &work,
            &destination.join("boundary.json"),
            super::boundary::MAX_LOG_BYTES,
        );
        command
            .args(&request.command[1..])
            .current_dir(&cwd)
            .env_clear()
            .env("LANG", "C")
            .stdin(
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(destination.join("execution.json"))?,
            )
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
            stop(&mut child)?;
        }
        outcome
    })();
    let outcome = match outcome {
        Err(error) if error.kind() == io::ErrorKind::TimedOut => return Err(error),
        other => other,
    };
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
    let execution_path = destination.join("execution.json");
    if execution_path.metadata().is_ok_and(|meta| meta.len() != 0) {
        let report: ExecutionReport = read_json(&execution_path)?;
        receipt.execution_exit = report.exit_code;
        receipt.execution_signal = report.signal;
        if receipt.status == RunStatus::Passed
            && (report.exit_code != Some(0)
                || report.signal.is_some()
                || !report.descendants_drained)
        {
            receipt.status = RunStatus::Failed;
            receipt.error = Some("native execution report did not confirm producer success".into());
        }
    } else if receipt.status == RunStatus::Passed {
        return Err(invalid("native executor did not retain execution status"));
    }
    if cancelled.load(Ordering::Relaxed) {
        receipt.status = RunStatus::Cancelled;
    } else if receipt.status == RunStatus::Failed
        && started.elapsed() >= Duration::from_secs(request.timeout_seconds)
    {
        receipt.status = RunStatus::TimedOut;
    }
    if ["stdout.log", "stderr.log"].into_iter().any(|name| {
        fs::metadata(destination.join(name))
            .is_ok_and(|metadata| metadata.len() >= super::boundary::MAX_LOG_BYTES)
    }) {
        receipt.status = RunStatus::Failed;
        receipt.error = Some("producer reached the 512 MiB per-stream output limit".into());
    }
    match capture(request, &mut check) {
        Ok((after, _)) => {
            if after != receipt.composition {
                receipt.status = RunStatus::SourceChanged;
            }
            receipt.after = Some(after);
        }
        Err(error) => {
            receipt.status = match error.kind() {
                io::ErrorKind::Interrupted => RunStatus::Cancelled,
                io::ErrorKind::TimedOut => RunStatus::TimedOut,
                _ => RunStatus::SourceChanged,
            };
            receipt.error = Some(error.to_string());
        }
    }
    // Tool and log finalization share a single finite cleanup budget, including
    // the failed/cancelled case where the producer's execution deadline elapsed.
    write_json(
        &destination.join("finalizing.json"),
        &serde_json::json!({
            "phase": "finalizing", "status": receipt.status, "after": receipt.after,
        }),
    )?;
    let finalize_deadline = Instant::now() + Duration::from_secs(5);
    let mut finalize = || {
        if cancelled.load(Ordering::Relaxed) {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "evidence finalization cancelled",
            ))
        } else if Instant::now() < finalize_deadline {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "evidence finalization deadline exceeded",
            ))
        }
    };
    let finalization = (|| -> io::Result<()> {
        for tool in receipt.tools.values() {
            if sha256_file_checked(&tool.path, &mut finalize).ok().as_ref() != Some(&tool.sha256) {
                receipt.status = RunStatus::SourceChanged;
            }
        }
        for tool in receipt.containment.values() {
            if sha256_file_checked(&tool.path, &mut finalize).ok().as_ref() != Some(&tool.sha256) {
                receipt.status = RunStatus::SourceChanged;
            }
        }
        if let Some(executable) = &receipt.executable {
            if sha256_file_checked(&executable.path, &mut finalize)
                .ok()
                .as_ref()
                != Some(&executable.sha256)
            {
                receipt.status = RunStatus::SourceChanged;
            }
        }
        if execution_path.exists() {
            receipt.execution_report_sha256 =
                Some(sha256_file_checked(&execution_path, &mut finalize)?);
        }
        receipt.stdout_sha256 =
            sha256_file_checked(&destination.join("stdout.log"), &mut finalize)?;
        receipt.stderr_sha256 =
            sha256_file_checked(&destination.join("stderr.log"), &mut finalize)?;
        finalize()?;
        Ok(())
    })();
    if let Err(error) = finalization {
        receipt.status = match error.kind() {
            io::ErrorKind::Interrupted => RunStatus::Cancelled,
            io::ErrorKind::TimedOut => RunStatus::TimedOut,
            _ => RunStatus::Failed,
        };
        receipt.error = Some(error.to_string());
    }
    if receipt.status == RunStatus::Passed && Instant::now() >= deadline {
        receipt.status = RunStatus::TimedOut;
    }
    if cancelled.load(Ordering::Relaxed) {
        receipt.status = RunStatus::Cancelled;
    }
    publish_receipt(&destination, &mut receipt, started, deadline, cancelled)?;
    Ok(receipt)
}

// Publish only after cooperative serialization, bounded writes and durable sync.
// Cancellation/expiry during publication discards the staged passing document;
// a retry may retain a non-accepting disposition within a fresh cleanup budget.
fn publish_receipt(
    directory: &Path,
    receipt: &mut Receipt,
    started: Instant,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    let cleanup = Instant::now() + Duration::from_secs(5);
    let staged = directory.join("receipt.pending.json");
    loop {
        let passed = receipt.status == RunStatus::Passed;
        let mut check = || {
            if passed && cancelled.load(Ordering::Relaxed) {
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "receipt publication cancelled",
                ))
            } else if Instant::now()
                >= if passed {
                    deadline.min(cleanup)
                } else {
                    cleanup
                }
            {
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "receipt publication deadline exceeded",
                ))
            } else {
                Ok(())
            }
        };
        let outcome = (|| -> io::Result<()> {
            // A fixed-width numeric slot permits a final elapsed-time update
            // without serializing the entire inventory again after disk sync.
            receipt.elapsed_ms = u128::MAX;
            let mut bytes = Vec::new();
            {
                let mut writer = CheckedWriter {
                    inner: &mut bytes,
                    check: &mut check,
                    count: 0,
                };
                serde_json::to_writer_pretty(&mut writer, &receipt).map_err(|error| {
                    io::Error::new(
                        error.io_error_kind().unwrap_or(io::ErrorKind::InvalidData),
                        error,
                    )
                })?;
                writer.write_all(b"\n")?;
            }
            let marker = format!("\"elapsedMs\": {}", u128::MAX);
            let offset = bytes
                .windows(marker.len())
                .rposition(|window| window == marker.as_bytes())
                .ok_or_else(|| invalid("missing elapsed time slot"))?
                + "\"elapsedMs\": ".len();
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged)?;
            for chunk in bytes.chunks(65536) {
                check()?;
                file.write_all(chunk)?;
            }
            file.sync_all()?;
            check()?;
            receipt.elapsed_ms = started.elapsed().as_millis();
            let elapsed = format!("{:<39}", receipt.elapsed_ms);
            file.seek(SeekFrom::Start(offset as u64))?;
            file.write_all(elapsed.as_bytes())?;
            file.sync_all()?;
            check()?;
            fs::rename(&staged, directory.join("receipt.json"))?;
            Ok(())
        })();
        match outcome {
            Ok(()) => return Ok(()),
            Err(error) => {
                if staged.exists() {
                    fs::remove_file(&staged)?;
                }
                if !passed {
                    return Err(error);
                }
                receipt.status = match error.kind() {
                    io::ErrorKind::Interrupted => RunStatus::Cancelled,
                    io::ErrorKind::TimedOut => RunStatus::TimedOut,
                    _ => RunStatus::Failed,
                };
                receipt.error = Some(error.to_string());
            }
        }
    }
}

struct CheckedWriter<'a, W, F> {
    inner: &'a mut W,
    check: &'a mut F,
    count: usize,
}
impl<W: io::Write, F: FnMut() -> io::Result<()>> io::Write for CheckedWriter<'_, W, F> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        (self.check)()?;
        if self.count.saturating_add(bytes.len()) > 64 * 1024 * 1024 {
            return Err(invalid("evidence JSON exceeds 64 MiB"));
        }
        let written = self.inner.write(bytes)?;
        self.count += written;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        (self.check)()?;
        self.inner.flush()
    }
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        // Write's default implementation retries Interrupted indefinitely.
        // Cooperative cancellation must escape serialization, not be retried.
        (self.check)()?;
        if self.count.saturating_add(bytes.len()) > 64 * 1024 * 1024 {
            return Err(invalid("evidence JSON exceeds 64 MiB"));
        }
        self.inner.write_all(bytes)?;
        self.count += bytes.len();
        Ok(())
    }
}

/// Verify completed receipt integrity and, when supplied, the required immutable composition.
/// This is portable evidence verification; it does not rerun commands or trust signatures.
pub fn verify_run(directory: &Path, expected: Option<&Composition>) -> io::Result<Receipt> {
    verify_run_bound(directory, expected, None)
}

pub(crate) fn verify_run_bound(
    directory: &Path,
    expected: Option<&Composition>,
    receipt_sha256: Option<&str>,
) -> io::Result<Receipt> {
    let root = directory.canonicalize()?;
    for name in [
        "receipt.json",
        "request.json",
        "execution.json",
        "stdout.log",
        "stderr.log",
    ] {
        evidence_path(&root, Path::new(name))?;
    }
    let directory = &root;
    let receipt: Receipt = read_bound_json(&directory.join("receipt.json"), receipt_sha256)?;
    if receipt.schema_version != 5
        || receipt.composition.schema_version != 1
        || receipt.status != RunStatus::Passed
        || receipt.execution_exit != Some(0)
        || receipt.execution_signal.is_some()
        || receipt.after.as_ref() != Some(&receipt.composition)
        || receipt.error.is_some()
        || receipt.composition.sources.is_empty()
        || receipt.executable.is_none()
        || receipt.containment.len() != 3
        || !receipt.containment.contains_key("unshare")
        || !receipt.containment.contains_key("executor")
        || !receipt.containment.contains_key("setpriv")
        || expected.is_some_and(|expected| expected != &receipt.composition)
    {
        return Err(invalid(
            "receipt is incomplete, failed, changed, or belongs to another composition",
        ));
    }
    let valid_digest =
        |hash: &str| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit());
    if receipt
        .executable
        .iter()
        .chain(receipt.containment.values())
        .any(|program| !valid_digest(&program.sha256))
        || receipt
            .tools
            .values()
            .any(|tool| !valid_digest(&tool.sha256))
    {
        return Err(invalid("receipt program identity is not a SHA-256 digest"));
    }
    let report_path = directory.join("execution.json");
    let report: ExecutionReport = read_bound_json(
        &report_path,
        Some(
            receipt
                .execution_report_sha256
                .as_deref()
                .ok_or_else(|| invalid("missing native execution report identity"))?,
        ),
    )?;
    if report.exit_code != Some(0) || report.signal.is_some() || !report.descendants_drained {
        return Err(invalid("native execution report did not confirm success"));
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
        ("stdout.log", &receipt.stdout_sha256),
        ("stderr.log", &receipt.stderr_sha256),
    ] {
        if sha256_file(&directory.join(name))? != *hash {
            return Err(invalid(format!("receipt artifact changed: {name}")));
        }
    }
    let request: RunRequest = read_bound_json(
        &directory.join("request.json"),
        Some(&receipt.request_sha256),
    )?;
    validate_runtime_selection(&request)?;
    if request.command.is_empty()
        || request.command[0].is_empty()
        || request.argument_zero.as_ref().is_some_and(String::is_empty)
        || !(1..=86400).contains(&request.timeout_seconds)
        || request.tools.keys().ne(receipt.tools.keys())
        || request.tools.keys().ne(request.tool_argument_zeros.keys())
        || request.tool_argument_zeros.values().any(String::is_empty)
        || request.sources.keys().ne(receipt.inventories.keys())
        || request.inputs.keys().ne(receipt.composition.inputs.keys())
        || request.environment != receipt.environment
    {
        return Err(invalid("receipt requirements differ from retained request"));
    }
    let programs = request
        .containment
        .as_ref()
        .ok_or_else(|| invalid("retained request lacks containment"))?;
    // Retained requests carry canonical selections. Verification is portable and
    // never resolves them on the machine reading the evidence.
    for (name, path) in [
        ("unshare", &programs.unshare),
        ("executor", &programs.executor),
        ("setpriv", &programs.setpriv),
    ] {
        if receipt.containment.get(name).map(|identity| &identity.path) != Some(path) {
            return Err(invalid(
                "receipt containment selection differs from retained request",
            ));
        }
    }
    if receipt.executable.as_ref().map(|identity| &identity.path)
        != Some(&PathBuf::from(&request.command[0]))
        || request
            .tools
            .iter()
            .any(|(name, path)| receipt.tools.get(name).map(|tool| &tool.path) != Some(path))
    {
        return Err(invalid(
            "receipt tool selection differs from retained request",
        ));
    }
    Ok(receipt)
}

#[cfg(test)]
mod publication_tests {
    use super::*;

    #[test]
    fn cancellation_is_observed_midway_through_json_serialization() {
        let value = vec!["inventory-entry"; 4096];
        let mut buffer = Vec::new();
        let mut checks = 0;
        let mut check = || {
            checks += 1;
            if checks >= 100 {
                Err(io::Error::from(io::ErrorKind::Interrupted))
            } else {
                Ok(())
            }
        };
        let mut writer = CheckedWriter {
            inner: &mut buffer,
            check: &mut check,
            count: 0,
        };
        let error = serde_json::to_writer_pretty(&mut writer, &value).unwrap_err();
        assert_eq!(error.io_error_kind(), Some(io::ErrorKind::Interrupted));
        assert!(!buffer.is_empty());
        assert!(buffer.len() < json_bytes(&value).unwrap().len());
    }
}
