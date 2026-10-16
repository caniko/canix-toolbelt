use super::*;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_TOOLS: usize = 32;
const MAX_TOOL_VERSION_BYTES: usize = 8 * 1024 * 1024;

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
    /// SHA-256 of the exact boundary policy admitted by every contained execution.
    pub boundary_sha256: String,
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
    for inventory in inventories.values() {
        validate_source_links(inventory)?;
    }
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

fn validate_source_links(inventory: &Inventory) -> io::Result<()> {
    for (name, file) in &inventory.files {
        if let Some(link) = &file.link {
            let (target, _) = super::seal::link_walk(name, Path::new(link))?;
            if !inventory.files.contains_key(&target) {
                return Err(invalid(
                    "source symlink target requires an explicit selection in the same inventory",
                ));
            }
        }
    }
    Ok(())
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
            // Setup may not yet consult PID 1's cancellation flag. Kill and
            // reap unshare too: --kill-child=KILL then tears down its namespace.
            // Forced cleanup never permits final capture or a passing receipt.
            child.kill()?;
            child.wait()?;
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

// Bootstrap executes before PID 1 can create the private root. Keep its mutable
// programs in sealed, digest-checked anonymous files until every launch is reaped.
struct Bootstrap {
    identities: BTreeMap<String, ProgramIdentity>,
    snapshots: BTreeMap<String, File>,
}

impl Bootstrap {
    fn new(
        identities: &BTreeMap<String, ProgramIdentity>,
        check: &mut impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (identities, check);
            Err(invalid("qualification bootstrap requires Linux"))
        }
        #[cfg(target_os = "linux")]
        {
            let mut snapshots = BTreeMap::new();
            for name in ["unshare", "executor"] {
                use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};
                use std::{io::Write, os::unix::fs::PermissionsExt};
                let identity = &identities[name];
                // Preserve the original executable attachment for platform security
                // policy (notably AppArmor userns admission). Only a non-root caller
                // beneath an entirely root-owned, non-writable path may use this route.
                if root_owned_program(&identity.path)? {
                    continue;
                }
                check()?;
                let input = regular_file(&identity.path)?;
                let length = input.metadata()?.len();
                if length > 256 * 1024 * 1024 {
                    return Err(invalid("bootstrap program exceeds 256 MiB"));
                }
                // Deliberately inherited through unshare, including script launchers.
                // PID 1 closes every descriptor >= 3 before any producer is started.
                let mut output = File::from(memfd_create(
                    name,
                    MemfdFlags::ALLOW_SEALING | MemfdFlags::EXEC,
                )?);
                let mut input = input.take(length + 1);
                let mut buffer = [0; 65536];
                loop {
                    check()?;
                    let count = input.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    output.write_all(&buffer[..count])?;
                }
                if input.limit() != 1 {
                    return Err(invalid("bootstrap program changed while copying"));
                }
                output.set_permissions(fs::Permissions::from_mode(0o500))?;
                fcntl_add_seals(
                    &output,
                    SealFlags::WRITE | SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL,
                )?;
                // Hash after sealing: another same-UID process cannot modify the
                // selected bytes between this check and either exec, even via proc-fd.
                if sha256_file_checked(&Self::descriptor_path(&output), check)? != identity.sha256 {
                    return Err(invalid("bootstrap snapshot differs from selected program"));
                }
                snapshots.insert(name.into(), output);
            }
            Ok(Self {
                identities: identities.clone(),
                snapshots,
            })
        }
    }

    fn descriptor_path(file: &File) -> PathBuf {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
        }
        #[cfg(not(unix))]
        {
            let _ = file;
            PathBuf::new()
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.snapshots
            .get(name)
            .map(Self::descriptor_path)
            .unwrap_or_else(|| self.identities[name].path.clone())
    }
}

#[cfg(target_os = "linux")]
fn root_owned_program(path: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    if rustix::process::geteuid().is_root() {
        return Ok(false);
    }
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        if metadata.uid() != 0
            || metadata.mode() & 0o022 != 0
            || rustix::fs::access(ancestor, rustix::fs::Access::WRITE_OK).is_ok()
            || metadata.file_type().is_symlink()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn contained_command(
    tools: &Bootstrap,
    executable: &Path,
    argument_zero: &std::ffi::OsStr,
    work: &Path,
    boundary: &Path,
    boundary_sha256: &str,
    max_log_bytes: u64,
) -> Command {
    let mut command = Command::new(tools.path("unshare"));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.arg0(&tools.identities["unshare"].path);
    }
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
        .arg(tools.path("executor"))
        .arg(work)
        .arg(boundary)
        .arg(boundary_sha256)
        .arg(max_log_bytes.to_string())
        .arg(&tools.identities["setpriv"].path)
        .arg(&tools.identities["executor"].path)
        .arg(executable)
        .arg(argument_zero);
    command
}

fn tool_version(
    tool: (&str, &Path, &str),
    request: &RunRequest,
    cancelled: &AtomicBool,
    directory: &Path,
    deadline: Instant,
    receipt: &Receipt,
    bootstrap: &Bootstrap,
) -> io::Result<Vec<u8>> {
    let (name, program, argument_zero) = tool;
    let name = format!("tool-{name}.log");
    let path = directory.join(&name);
    let stderr_path = directory.join(format!("{name}.stderr"));
    let mut command = contained_command(
        bootstrap,
        program,
        std::ffi::OsStr::new(argument_zero),
        &directory.join("work"),
        &directory.join("boundary.json"),
        &receipt.boundary_sha256,
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
    let mut output = Vec::new();
    for stream in [&path, &stderr_path] {
        let file = regular_file(stream)?;
        let length = file.metadata()?.len();
        if length > 1024 * 1024 {
            return Err(invalid("tool version stream exceeds 1 MiB"));
        }
        let start = output.len();
        file.take(1024 * 1024 + 1).read_to_end(&mut output)?;
        if output.len() - start != length as usize {
            return Err(invalid("tool version stream changed during bounded read"));
        }
    }
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
    if request.tools.len() > MAX_TOOLS {
        return Err(invalid("qualification supports at most 32 tool probes"));
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
    let bootstrap = Bootstrap::new(&containment, &mut check)?;
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
    let selected_tools = effective_request
        .tools
        .iter()
        .map(|(name, path)| {
            Ok((
                name.clone(),
                ProgramIdentity {
                    path: path.clone(),
                    sha256: sha256_file_checked(path, &mut check)?,
                },
            ))
        })
        .collect::<io::Result<BTreeMap<_, _>>>()?;
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
    let read_only_paths: Vec<PathBuf> = super::boundary::SYSTEM_PATHS
        .iter()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .collect();
    let mut snapshot_mounts = Vec::new();
    let mut snapshot_files = BTreeMap::new();
    let mut snapshot_directories = Vec::new();
    for (name, spec) in &request.sources {
        for path in &spec.paths {
            snapshot_mounts.push(spec.root.join(path));
        }
        for directory in super::seal::source_directories(&inventories[name].files)? {
            snapshot_directories.push(spec.root.join(directory));
        }
        for (path, file) in &inventories[name].files {
            insert_snapshot(&mut snapshot_files, spec.root.join(path), file.clone())?;
        }
    }
    for (name, path) in &request.inputs {
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            fs::metadata(path)?.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        insert_snapshot(
            &mut snapshot_files,
            path.clone(),
            SourceFile {
                sha256: composition.inputs[name].clone(),
                executable,
                link: None,
            },
        )?;
        snapshot_mounts.push(path.clone());
    }
    for program in selected_tools
        .values()
        .chain(containment.values())
        .chain(executable.as_ref().ok())
    {
        insert_snapshot(
            &mut snapshot_files,
            program.path.clone(),
            SourceFile {
                sha256: program.sha256.clone(),
                executable: true,
                link: None,
            },
        )?;
        snapshot_mounts.push(program.path.clone());
    }
    snapshot_mounts.extend(request.runtime_paths.iter().cloned());
    if read_only_paths
        .iter()
        .chain(&snapshot_mounts)
        .any(|path| !path.is_absolute() || path == Path::new("/"))
    {
        return Err(invalid(
            "runtime mounts must be absolute paths below the host root",
        ));
    }
    let boundary = super::boundary::Boundary {
        read_only_paths,
        snapshot_mounts,
        snapshot_files,
        snapshot_directories,
        environment: environment.clone(),
        sandbox,
    };
    // Select the bytes from trusted memory, never by re-hashing a writable path
    // after publication. PID 1 must admit this identity before parsing or mounting.
    let boundary_sha256 = fingerprint(&boundary)?;
    write_json(&destination.join("boundary.json"), &boundary)?;
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
        boundary_sha256,
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
        let mut version_bytes = 0usize;
        for (name, tool) in &selected_tools {
            relative(Path::new(name))?;
            if Path::new(name).components().count() != 1 {
                return Err(invalid("tool labels must be single path components"));
            }
            let output = tool_version(
                (name, &tool.path, &request.tool_argument_zeros[name]),
                request,
                cancelled,
                &destination,
                started + Duration::from_secs(request.timeout_seconds),
                &receipt,
                &bootstrap,
            )?;
            version_bytes = version_bytes.saturating_add(output.len());
            if version_bytes > MAX_TOOL_VERSION_BYTES {
                return Err(invalid("aggregate tool version evidence exceeds 8 MiB"));
            }
            let version = String::from_utf8(output).map_err(|e| invalid(e.to_string()))?;
            receipt.tools.insert(
                name.clone(),
                ToolEvidence {
                    path: tool.path.clone(),
                    sha256: tool.sha256.clone(),
                    version,
                },
            );
        }
        let executable = executable?;
        let mut command = contained_command(
            &bootstrap,
            &executable.path,
            std::ffi::OsStr::new(
                request
                    .argument_zero
                    .as_deref()
                    .unwrap_or(&request.command[0]),
            ),
            &work,
            &destination.join("boundary.json"),
            &receipt.boundary_sha256,
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
        if sha256_file_checked(&destination.join("boundary.json"), &mut finalize)
            .ok()
            .as_ref()
            != Some(&receipt.boundary_sha256)
        {
            receipt.status = RunStatus::SourceChanged;
            receipt.error = Some("boundary policy changed during execution".into());
        }
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

fn insert_snapshot(
    files: &mut BTreeMap<PathBuf, SourceFile>,
    path: PathBuf,
    file: SourceFile,
) -> io::Result<()> {
    if files.get(&path).is_some_and(|selected| selected != &file) {
        return Err(invalid(
            "overlapping snapshot selections have different identities",
        ));
    }
    files.insert(path, file);
    Ok(())
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
        "boundary.json",
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
        .composition
        .inputs
        .values()
        .any(|hash| !valid_digest(hash))
    {
        return Err(invalid("receipt input identity is not a SHA-256 digest"));
    }
    if receipt
        .executable
        .iter()
        .chain(receipt.containment.values())
        .any(|program| !valid_digest(&program.sha256))
        || receipt
            .tools
            .values()
            .any(|tool| !valid_digest(&tool.sha256))
        || !valid_digest(&receipt.boundary_sha256)
    {
        return Err(invalid(
            "receipt program or boundary identity is not a SHA-256 digest",
        ));
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
            || inventory
                .files
                .values()
                .any(|file| !valid_digest(&file.sha256))
            || receipt.composition.sources.get(name) != Some(&fingerprint(inventory)?)
        {
            return Err(invalid("receipt source inventory identity differs"));
        }
        validate_source_links(inventory)?;
    }
    if receipt.tools.len() > MAX_TOOLS
        || receipt.tools.values().fold(0usize, |total, tool| {
            total.saturating_add(tool.version.len())
        }) > MAX_TOOL_VERSION_BYTES
    {
        return Err(invalid("receipt exceeds aggregate tool version limits"));
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
    let boundary: super::boundary::Boundary = read_bound_json(
        &directory.join("boundary.json"),
        Some(&receipt.boundary_sha256),
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
        || boundary.environment != receipt.environment
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
    let mut files = BTreeMap::new();
    let mut mounts = Vec::new();
    let mut directories = Vec::new();
    for (name, spec) in &request.sources {
        for path in &spec.paths {
            mounts.push(spec.root.join(path));
        }
        for directory in super::seal::source_directories(&receipt.inventories[name].files)? {
            directories.push(spec.root.join(directory));
        }
        for (path, file) in &receipt.inventories[name].files {
            insert_snapshot(&mut files, spec.root.join(path), file.clone())?;
        }
    }
    for (name, path) in &request.inputs {
        let selected = boundary
            .snapshot_files
            .get(path)
            .ok_or_else(|| invalid("missing input snapshot"))?;
        insert_snapshot(
            &mut files,
            path.clone(),
            SourceFile {
                sha256: receipt.composition.inputs[name].clone(),
                // Inputs bind bytes, not a Unix mode. The boundary retains its
                // effective executable bit without consulting the original host.
                executable: selected.executable,
                link: None,
            },
        )?;
        mounts.push(path.clone());
    }
    for (path, hash) in receipt
        .tools
        .values()
        .map(|tool| (&tool.path, &tool.sha256))
        .chain(
            receipt
                .containment
                .values()
                .chain(receipt.executable.iter())
                .map(|program| (&program.path, &program.sha256)),
        )
    {
        insert_snapshot(
            &mut files,
            path.clone(),
            SourceFile {
                sha256: hash.clone(),
                executable: true,
                link: None,
            },
        )?;
        mounts.push(path.clone());
    }
    mounts.extend(request.runtime_paths.iter().cloned());
    mounts.sort();
    mounts.dedup();
    directories.sort();
    directories.dedup();
    let mut actual_mounts = boundary.snapshot_mounts.clone();
    actual_mounts.sort();
    actual_mounts.dedup();
    let mut actual_directories = boundary.snapshot_directories.clone();
    actual_directories.sort();
    actual_directories.dedup();
    if files != boundary.snapshot_files
        || mounts != actual_mounts
        || directories != actual_directories
        || boundary.read_only_paths.iter().any(|path| {
            !super::boundary::SYSTEM_PATHS
                .iter()
                .any(|system| path == Path::new(system))
        })
    {
        return Err(invalid(
            "boundary snapshots differ from the retained composition and program selections",
        ));
    }
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn bootstrap_seals_both_launchers_before_path_or_inode_substitution() {
        use std::os::unix::fs::PermissionsExt;
        for replace in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let unshare = root.path().join("unshare");
            let executor = root.path().join("executor");
            fs::write(
                &unshare,
                b"#!/bin/sh\nwhile test \"$1\" != --; do shift; done\nshift\nexec \"$@\"\n",
            )
            .unwrap();
            fs::write(&executor, b"#!/bin/sh\nprintf approved\n").unwrap();
            let identities: BTreeMap<_, _> = [
                ("unshare", &unshare),
                ("executor", &executor),
                ("setpriv", &executor),
            ]
            .into_iter()
            .map(|(name, path)| {
                fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
                (
                    name.into(),
                    ProgramIdentity {
                        path: path.clone(),
                        sha256: sha256_file(path).unwrap(),
                    },
                )
            })
            .collect();
            let bootstrap = Bootstrap::new(&identities, &mut || Ok(())).unwrap();
            assert_eq!(bootstrap.snapshots.len(), 2);
            for file in bootstrap.snapshots.values() {
                assert!(fs::write(Bootstrap::descriptor_path(file), b"forged").is_err());
                assert!(file.set_len(0).is_err());
            }
            for path in [&unshare, &executor] {
                let target = if replace {
                    path.with_extension("swap")
                } else {
                    path.clone()
                };
                fs::write(&target, b"#!/bin/sh\nprintf forged\n").unwrap();
                fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
                if replace {
                    fs::rename(target, path).unwrap();
                }
            }
            let output = contained_command(
                &bootstrap,
                Path::new("/producer"),
                std::ffi::OsStr::new("producer"),
                root.path(),
                &root.path().join("boundary.json"),
                &"a".repeat(64),
                1,
            )
            .env_clear()
            .output()
            .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.stdout, b"approved", "replace={replace}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stop_reaps_uncooperative_launcher_after_grace_period() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "trap '' TERM; printf ready; exec sleep 60"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut ready = [0; 5];
        child.stdout.take().unwrap().read_exact(&mut ready).unwrap();
        assert_eq!(&ready, b"ready");
        let error = stop(&mut child).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        let exited = child.try_wait().unwrap();
        // Reap a leaked baseline child before failing the regression itself.
        if exited.is_none() {
            child.kill().unwrap();
            child.wait().unwrap();
        }
        assert!(
            exited.is_some(),
            "cleanup returned with the launcher still alive"
        );
    }

    #[test]
    fn huge_sparse_post_execution_inputs_respect_the_deadline() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("source"), b"source").unwrap();
        let input = root.path().join("input");
        fs::write(&input, b"pinned").unwrap();
        let request: RunRequest = serde_json::from_value(serde_json::json!({
            "cwd": root.path(),
            "sources": {"fixture": {"root": root.path(), "paths": ["source"]}},
            "inputs": {"input": input},
            "command": ["true"],
            "timeoutSeconds": 1,
        }))
        .unwrap();
        capture(&request, &mut || Ok(())).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&input)
            .unwrap()
            .set_len(1 << 40)
            .unwrap();
        // Exercise the actual post-execution capture/hash path without spending
        // its entire budget on a cold namespace launch. No deadline is enlarged.
        let started = Instant::now();
        let deadline = started + Duration::from_secs(request.timeout_seconds);
        let error = capture(&request, &mut || {
            if Instant::now() >= deadline {
                Err(io::Error::from(io::ErrorKind::TimedOut))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(4));
    }

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
