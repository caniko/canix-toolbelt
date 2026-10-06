//! Offline controller fixture; excluded from production packages.
use anyhow::{Context, Result, ensure};
use canix_toolbelt_roborev_worker::admission::{Admission, AdmissionBinding, JobIdentity};
use canix_toolbelt_roborev_worker::peer::DirectAdapterAuthority;
use std::{
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    time::{Duration, Instant},
};

fn send_frame(stream: &mut UnixStream, bytes: &[u8]) -> Result<()> {
    ensure!(bytes.len() <= 2 * 1024 * 1024, "oversized fixture frame");
    stream.write_all(&(bytes.len() as u32).to_le_bytes())?;
    stream.write_all(bytes)?;
    Ok(())
}

fn read_frame(stream: &mut UnixStream) -> Result<Vec<u8>> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    ensure!(length <= 2 * 1024 * 1024, "oversized fixture frame");
    let mut bytes = vec![0u8; length];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn connection(path: &std::path::Path) -> Result<UnixStream> {
    let stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    Ok(stream)
}

fn peer_fixture(mut args: impl Iterator<Item = std::ffi::OsString>) -> Result<()> {
    let pid: u32 = args
        .next()
        .context("daemon PID required")?
        .to_str()
        .context("invalid PID")?
        .parse()?;
    let executable = PathBuf::from(args.next().context("adapter image required")?);
    let endpoint = PathBuf::from(args.next().context("fixture endpoint required")?);
    let handle = PathBuf::from(args.next().context("original admission required")?);
    let receipt = PathBuf::from(args.next().context("fixture receipt required")?);
    ensure!(args.next().is_none(), "unexpected peer fixture argument");
    let admission: Admission = serde_json::from_slice(&std::fs::read(handle)?)?;
    let job = admission.dispatched_job()?;
    let authority = DirectAdapterAuthority::capture(pid, &executable)?;
    let listener = UnixListener::bind(&endpoint)?;
    std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    println!("ready");
    std::io::stdout().flush()?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(error).context("bounded fixture accept"),
        }
    };
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let authenticated = authority.authenticate(&stream)?;
    let bytes = read_frame(&mut stream)?;
    let invocation: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure!(
        invocation["arguments"]
            == serde_json::json!(["run", "--format", "json", "--model", "fixture/admission"]),
        "fixture invocation policy differs"
    );
    let arguments = authenticated.arguments()?;
    ensure!(
        arguments[1..] == ["run", "--format", "json", "--model", "fixture/admission"],
        "kernel-observed invocation policy differs"
    );
    let actual_checkout = authenticated.checkout_directory()?.metadata()?;
    let claimed_checkout =
        std::fs::metadata(invocation["cwd"].as_str().context("invalid cwd claim")?)?;
    use std::os::unix::fs::MetadataExt;
    ensure!(
        (actual_checkout.dev(), actual_checkout.ino())
            == (claimed_checkout.dev(), claimed_checkout.ino()),
        "delivery checkout differs from kernel cwd"
    );
    authenticated.check_live()?;
    ensure!(
        admission.dispatched_job()? == job,
        "original admitted job changed"
    );
    let retained = serde_json::json!({"job": job, "daemon": authority.daemon_identity(), "adapter": authenticated.identity(), "invocation": invocation});
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(receipt)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(&serde_json::to_vec(&retained)?)?;
    file.sync_all()?;
    let document = serde_json::json!({"schema_version": 2, "summary": "OFFLINE_ADMISSION_REVIEW", "verdict": "pass", "findings": []});
    let event =
        serde_json::json!({"type": "text", "part": {"type": "text", "text": document.to_string()}});
    send_frame(&mut stream, format!("{event}\n").as_bytes())?;
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let command = args.next().context("fixture command required")?;
    if command == "authenticate-fd" {
        use std::os::fd::FromRawFd;
        let pid: u32 = args
            .next()
            .context("daemon PID required")?
            .to_str()
            .context("invalid PID")?
            .parse()?;
        let executable = PathBuf::from(args.next().context("adapter image required")?);
        let descriptor: i32 = args
            .next()
            .context("inherited socket required")?
            .to_str()
            .context("invalid socket FD")?
            .parse()?;
        ensure!(
            descriptor >= 3 && args.next().is_none(),
            "invalid inherited fixture socket"
        );
        // SAFETY: this test-only mode accepts one explicitly inherited owned Unix
        // socket from its namespace-isolated parent. It is absent from production.
        let stream = unsafe { UnixStream::from_raw_fd(descriptor) };
        let authority = DirectAdapterAuthority::capture(pid, &executable)?;
        let peer = authority.authenticate(&stream)?;
        println!("{}", serde_json::to_string(peer.identity())?);
        return Ok(());
    }
    if command == "peer" {
        return peer_fixture(args);
    }
    if command == "peer-client" || command == "peer-stale-client" {
        let endpoint = PathBuf::from(args.next().context("endpoint required")?);
        ensure!(args.next().is_none(), "unexpected client argument");
        let mut stream = connection(&endpoint)?;
        stream.write_all(b"x")?;
        if command == "peer-client" {
            stream.read_exact(&mut [0u8; 1])?;
        }
        return Ok(());
    }
    if command == "run" {
        let arguments: Vec<String> = std::iter::once(command)
            .chain(args)
            .map(|arg| {
                arg.into_string()
                    .map_err(|_| anyhow::anyhow!("non-UTF8 invocation"))
            })
            .collect::<Result<_>>()?;
        let endpoint = PathBuf::from(
            std::env::var_os("CANIX_ADMISSION_FIXTURE_PEER")
                .context("fixture peer endpoint required")?,
        );
        let mut prompt = Vec::new();
        std::io::stdin()
            .take(1024 * 1024 + 1)
            .read_to_end(&mut prompt)?;
        ensure!(
            !prompt.is_empty() && prompt.len() <= 1024 * 1024,
            "invalid fixture prompt"
        );
        let mut stream = connection(&endpoint)?;
        let invocation = serde_json::json!({"arguments": arguments, "cwd": std::env::current_dir()?, "prompt": String::from_utf8(prompt)?});
        send_frame(&mut stream, &serde_json::to_vec(&invocation)?)?;
        std::io::stdout().write_all(&read_frame(&mut stream)?)?;
        return Ok(());
    }
    let input = PathBuf::from(args.next().context("fixture input required")?);
    let bytes = std::fs::read(&input)?;
    if command == "register" {
        let state = PathBuf::from(args.next().context("state directory required")?);
        ensure!(args.next().is_none(), "unexpected fixture argument");
        let binding: AdmissionBinding = serde_json::from_slice(&bytes)?;
        println!(
            "{}",
            serde_json::to_string(&Admission::register(&state, &binding)?)?
        );
        return Ok(());
    }
    let admission: Admission = serde_json::from_slice(&bytes)?;
    match command.to_str().context("non-UTF8 fixture command")? {
        "enqueue-only" => {
            ensure!(args.next().is_none(), "unexpected fixture argument");
            admission.begin_enqueue()?;
            println!("effect-granted");
            return Ok(());
        }
        "reserve-only" => {
            ensure!(args.next().is_none(), "unexpected fixture argument");
            let _reservation = admission.reserve_execution()?;
            println!("effect-granted");
            return Ok(());
        }
        "enqueue" => admission.begin_enqueue()?,
        "dispatch" => admission.begin_dispatch()?,
        "bind" => {
            let path = PathBuf::from(args.next().context("job receipt required")?);
            let job: JobIdentity = serde_json::from_slice(&std::fs::read(path)?)?;
            admission.bind_job(&job)?;
        }
        "phase" => {}
        _ => anyhow::bail!("unknown fixture command"),
    }
    ensure!(args.next().is_none(), "unexpected fixture argument");
    println!("{}", serde_json::to_string(&admission.phase()?)?);
    Ok(())
}
