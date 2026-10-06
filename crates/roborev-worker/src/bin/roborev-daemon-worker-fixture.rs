//! Offline integration fixture, absent from production builds. All custody is
//! disposable and same-UID; this is not a production controller or broker.
use anyhow::{Context, Result, ensure};
use canix_toolbelt_roborev_worker::{
    Binding, Limits, Prepared, Tools,
    admission::{Admission, AdmissionBinding, AdmissionPhase, JobIdentity},
    execution::ExecutionBinding,
    offline::{Executable, OfflineSpec, WorkerLimits, run_offline},
    peer::DirectAdapterAuthority,
    prepare_local,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    git: PathBuf,
    systemd_run: PathBuf,
    systemctl: PathBuf,
    bubblewrap: PathBuf,
    helper: PathBuf,
    backend: PathBuf,
    daemon: PathBuf,
    store_paths: Vec<PathBuf>,
    script: String,
    limits: WorkerLimits,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    admission: Admission,
    prepared: Prepared,
    template: OfflineSpec,
    daemon: Executable,
    root: PathBuf,
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::File::open(path.parent().context("missing fixture parent")?)?.sync_all()?;
    Ok(())
}

fn initialize(config: &Path, binding: &Path, root: &Path, objects: &Path) -> Result<()> {
    let config: Config = serde_json::from_slice(&fs::read(config)?)?;
    let mut binding: AdmissionBinding = serde_json::from_slice(&fs::read(binding)?)?;
    binding.request.execution_policy_sha256 = digest(&serde_json::to_vec(&(
        1,
        "isolated-network-only",
        &config.limits,
    ))?);
    fs::DirBuilder::new().mode(0o700).create(root)?;
    for name in ["preparation", "admission", "execution", "completion"] {
        fs::DirBuilder::new().mode(0o700).create(root.join(name))?;
    }
    let tools = Tools {
        git: config.git,
        bubblewrap: config.bubblewrap.clone(),
        preparer: config.helper.clone(),
    };
    let prepared = prepare_local(
        objects,
        &root.join("preparation"),
        &binding.request,
        &tools,
        &Limits::default(),
    )?;
    let template = OfflineSpec::capture(
        Executable::capture(&config.systemd_run)?,
        Executable::capture(&config.systemctl)?,
        Executable::capture(&config.bubblewrap)?,
        Executable::capture(&config.helper)?,
        Executable::capture(&config.backend)?,
        vec!["/control/backend.py".into()],
        config.store_paths,
        prepared.checkout.clone(),
        BTreeMap::from([("backend.py".into(), config.script.into_bytes())]),
        vec![],
        config.limits,
    )?;
    ensure!(
        template.policy_digest()? == binding.request.execution_policy_sha256,
        "fixture's concrete worker policy differs from preparation admission"
    );
    let plan = Plan {
        admission: Admission::register(&root.join("admission"), &binding)?,
        prepared,
        template,
        daemon: Executable::capture(&config.daemon)?,
        root: root.to_owned(),
    };
    println!("{}", serde_json::to_string(&plan)?);
    Ok(())
}

fn connection(endpoint: &Path) -> Result<UnixStream> {
    let stream = UnixStream::connect(endpoint)?;
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    Ok(stream)
}

fn frame(stream: &mut UnixStream, bytes: &[u8]) -> Result<()> {
    ensure!(bytes.len() <= 2 * 1024 * 1024, "oversized fixture frame");
    stream.write_all(&(bytes.len() as u32).to_le_bytes())?;
    stream.write_all(bytes)?;
    Ok(())
}

fn read_frame(stream: &mut UnixStream) -> Result<Vec<u8>> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    ensure!(length <= 2 * 1024 * 1024, "oversized fixture frame");
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}

// A deliberately narrow fixture HTTP/1.0 close-delimited reader. This is not a
// reusable daemon transport; the fixture rejects transfer encodings entirely.
fn running_job(endpoint: &Path, plan: &Plan, job: &JobIdentity) -> Result<()> {
    let mut stream = connection(endpoint)?;
    stream.write_all(b"GET /api/jobs?limit=2&include_panel_members=true HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut bytes = Vec::new();
    stream.take(256 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 256 * 1024, "oversized fixture inventory");
    let split = bytes
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .context("incomplete fixture HTTP response")?;
    let headers = std::str::from_utf8(&bytes[..split])?;
    ensure!(
        headers.starts_with("HTTP/1.0 200 "),
        "fixture inventory failed"
    );
    let lengths: Vec<_> = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse::<usize>())
        .collect::<std::result::Result<_, _>>()?;
    ensure!(
        lengths.is_empty() || lengths == [bytes.len() - split - 4],
        "incomplete fixture inventory body"
    );
    ensure!(
        !headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .any(|(key, _)| key.eq_ignore_ascii_case("transfer-encoding")),
        "unexpected fixture transfer encoding"
    );
    let inventory: Value = serde_json::from_slice(&bytes[split + 4..])?;
    let jobs = inventory["jobs"]
        .as_array()
        .context("missing fixture jobs")?;
    ensure!(
        inventory["has_more"] == false && jobs.len() == 1,
        "exclusive fixture job inventory changed"
    );
    let original = &jobs[0];
    let request: &Binding = &plan.admission.binding().request;
    ensure!(
        original["id"] == job.id
            && original["uuid"] == job.uuid
            && original["status"] == "running"
            && original["git_ref"] == format!("{}..{}", request.base, request.head)
            && original["agent"] == "opencode"
            && original["model"] == "fixture/integrated"
            && original["job_type"] == "range",
        "original admitted job is not the running fixture comparison"
    );
    Ok(())
}

// The unchanged daemon relocates this generated context on each attempt. The
// fixture captures its actual bytes through the authenticated cwd descriptor
// before mapping ONE recognized reference into the private worker namespace.
// No other prompt change is normalized, and changed context bytes change the
// complete input digest. This same-UID fixture does not establish root custody.
fn capture_context(
    directory: &fs::File,
    cwd: &str,
    prompt: &str,
) -> Result<(String, Vec<u8>, Value)> {
    use nix::{
        fcntl::{OFlag, openat},
        sys::stat::Mode,
    };
    use std::os::unix::fs::MetadataExt;
    let marker = "<prior-range-reviews file=\"";
    let parts: Vec<_> = prompt.match_indices(marker).collect();
    ensure!(
        parts.len() == 1,
        "missing or ambiguous daemon context reference"
    );
    let start = parts[0].0 + marker.len();
    let end = start
        + prompt[start..]
            .find("\">\n")
            .context("invalid daemon context reference")?;
    let source = &prompt[start..end];
    let relative = source
        .strip_prefix(&format!("{cwd}/.roborev/"))
        .context("context is outside authenticated cwd")?;
    let (snapshot, member) = relative
        .split_once('/')
        .context("invalid context snapshot")?;
    let number = snapshot
        .strip_prefix("roborev-snapshot-")
        .context("unrecognized daemon context directory")?;
    ensure!(
        !number.is_empty()
            && number.bytes().all(|b| b.is_ascii_digit())
            && member == "prior-range-reviews.xml",
        "unexpected daemon context path"
    );
    let roborev = fs::File::from(openat(
        directory,
        ".roborev",
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?);
    let snapshot = fs::File::from(openat(
        &roborev,
        snapshot,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?);
    let mut file = fs::File::from(openat(
        &snapshot,
        member,
        OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?);
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == nix::unistd::geteuid().as_raw()
            && metadata.mode() & 0o022 == 0
            && metadata.len() <= 256 * 1024,
        "invalid daemon context member"
    );
    let mut bytes = Vec::new();
    (&mut file).take(256 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == metadata.len()
            && bytes.len() <= 256 * 1024
            && file.metadata()?.len() == metadata.len(),
        "daemon context changed or exceeded its bound"
    );
    let mut normalized = prompt.to_owned();
    normalized.replace_range(start..end, "/control/prior-range-reviews.xml");
    let observation = json!({"source_path": source, "source_device": metadata.dev(), "source_inode": metadata.ino(),
        "context_sha256": digest(&bytes), "raw_prompt_sha256": digest(prompt.as_bytes()), "worker_prompt_sha256": digest(normalized.as_bytes())});
    Ok((normalized, bytes, observation))
}

fn peer(
    plan: &Plan,
    pid: u32,
    api: &Path,
    endpoint: &Path,
    receipt: &Path,
    mode: &str,
) -> Result<()> {
    let job = plan.admission.dispatched_job()?;
    ensure!(
        digest(&fs::read(format!("/proc/{pid}/exe"))?) == plan.daemon.sha256,
        "fixture daemon differs from the qualified unchanged executable"
    );
    let authority = DirectAdapterAuthority::capture(pid, &std::env::current_exe()?)?;
    let listener = UnixListener::bind(endpoint)?;
    fs::set_permissions(endpoint, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(40);
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error).context("bounded fixture accept"),
        }
    };
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let authenticated = authority.authenticate(&stream)?;
    let invocation: Value = serde_json::from_slice(&read_frame(&mut stream)?)?;
    let expected = ["run", "--format", "json", "--model", "fixture/integrated"];
    ensure!(
        invocation["arguments"] == json!(expected),
        "claimed invocation changed"
    );
    ensure!(
        authenticated.arguments()?[1..] == expected,
        "kernel-observed invocation changed"
    );
    let directory = authenticated.checkout_directory()?;
    use std::os::unix::fs::MetadataExt;
    let actual = directory.metadata()?;
    let claimed = fs::metadata(invocation["cwd"].as_str().context("missing cwd")?)?;
    ensure!(
        (actual.dev(), actual.ino()) == (claimed.dev(), claimed.ino()),
        "adapter cwd claim differs"
    );
    // Only a disposable fixture file is asserted here, not arbitrary repository
    // equivalence or production job authority. The worker receives the independently
    // prepared exact-head snapshot, not a path submitted by the adapter.
    ensure!(
        fs::read(format!(
            "/proc/{}/cwd/fixture.txt",
            authenticated.identity().pid
        ))? == fs::read(plan.prepared.checkout.join("fixture.txt"))?,
        "fixture adapter checkout differs from the prepared head"
    );
    if mode == "capture-hold-peer" {
        write_new(
            &plan.root.join("capture-ready.json"),
            &serde_json::to_vec(&invocation)?,
        )?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while !plan.root.join("release-capture").exists() {
            ensure!(
                Instant::now() < deadline,
                "bounded fixture capture hold expired"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let mut spec = plan.template.clone();
    let (prompt, context, context_observation) = capture_context(
        &directory,
        invocation["cwd"].as_str().context("missing cwd")?,
        invocation["prompt"].as_str().context("missing prompt")?,
    )?;
    spec.prompt = prompt.into_bytes();
    spec.trusted_files
        .insert("prior-range-reviews.xml".into(), context);
    ensure!(!spec.prompt.is_empty(), "missing actual daemon input");
    let binding = ExecutionBinding {
        request: plan.admission.binding().request.clone(),
        controller_id: plan.admission.binding().controller_id.clone(),
        daemon_identity_sha256: plan.admission.binding().daemon_identity_sha256.clone(),
        job_id: job.id,
        job_uuid: job.uuid.clone(),
        execution_id: digest(plan.admission.binding().request.request_id.as_bytes()),
        input_manifest_sha256: spec.input_digest()?,
        backend_manifest_sha256: spec.backend_digest()?,
    };
    write_new(
        &receipt.with_extension("attempt.json"),
        &serde_json::to_vec(&json!({
            "scope": "offline invocation diagnostics; no completion or delivery attestation",
        "job": job, "binding": binding, "invocation": invocation,
        "context_observation": context_observation,
            "daemon": authority.daemon_identity(), "adapter": authenticated.identity()
        }))?,
    )?;
    if mode == "hold-peer" {
        write_new(
            &plan.root.join("hold-ready.json"),
            &serde_json::to_vec(&json!({
                "job": job, "adapter": authenticated.identity(), "binding": binding
            }))?,
        )?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while !plan.root.join("release-hold").exists() {
            ensure!(Instant::now() < deadline, "bounded fixture hold expired");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    running_job(api, plan, &job)?;
    authenticated.check_live()?;
    let recovered = plan.admission.phase()? == AdmissionPhase::CompletionRetained;
    if !recovered {
        plan.admission
            .register_execution(&plan.root.join("execution"), &binding)?;
        let result = plan.root.join("worker-result");
        let receipt = run_offline(plan.admission.reserve_execution()?, &spec, &result)?;
        plan.admission.retain_offline_completion(
            &plan.root.join("completion"),
            &result,
            &receipt,
        )?;
        write_new(
            &plan.root.join("original-execution.json"),
            &serde_json::to_vec(&binding)?,
        )?;
    }
    running_job(api, plan, &job)?;
    authenticated.check_live()?;
    let output = plan
        .admission
        .retained_offline_output(&binding, &job, true)?;
    write_new(
        receipt,
        &serde_json::to_vec(&json!({
            "scope": "offline-integrated-fixture; production authority unqualified",
            "job": job, "daemon": authority.daemon_identity(), "adapter": authenticated.identity(),
            "invocation": invocation, "binding": binding, "recovered": recovered,
            "context_observation": context_observation,
            "output_sha256": digest(&output), "reply_dropped": mode == "drop-peer"
        }))?,
    )?;
    ensure!(
        mode != "drop-peer",
        "deliberate fixture loss after retained completion"
    );
    frame(&mut stream, &output)?;
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, config, binding, root, objects] if command == "initialize" => initialize(
            Path::new(config),
            Path::new(binding),
            Path::new(root),
            Path::new(objects),
        ),
        [command, format, json, model, selected]
            if command == "run"
                && format == "--format"
                && json == "json"
                && model == "--model"
                && selected == "fixture/integrated" =>
        {
            let endpoint = std::env::var_os("CANIX_DAEMON_WORKER_FIXTURE_PEER")
                .context("private fixture endpoint missing")?;
            let mut prompt = Vec::new();
            std::io::stdin()
                .take(1024 * 1024 + 1)
                .read_to_end(&mut prompt)?;
            ensure!(
                !prompt.is_empty() && prompt.len() <= 1024 * 1024,
                "invalid fixture prompt"
            );
            let mut stream = connection(Path::new(&endpoint))?;
            frame(
                &mut stream,
                &serde_json::to_vec(
                    &json!({"arguments": args, "cwd": std::env::current_dir()?, "prompt": String::from_utf8(prompt)?}),
                )?,
            )?;
            std::io::stdout().write_all(&read_frame(&mut stream)?)?;
            Ok(())
        }
        [command, plan, pid, api, endpoint, receipt]
            if matches!(
                command.as_str(),
                "peer" | "drop-peer" | "hold-peer" | "capture-hold-peer"
            ) =>
        {
            let plan: Plan = serde_json::from_slice(&fs::read(plan)?)?;
            peer(
                &plan,
                pid.parse()?,
                Path::new(api),
                Path::new(endpoint),
                Path::new(receipt),
                command,
            )
        }
        _ => anyhow::bail!("unexpected offline fixture arguments"),
    }
}
