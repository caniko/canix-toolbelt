//! Request-local pinentry routing. PIN responses go straight to the caller on
//! Assuan stdout; the private Zellij socket transports only terminal metadata.

use std::env;
use std::ffi::OsString;
use std::fs::{self, DirBuilder};
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Seek, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ZELLIJ: &str = match option_env!("CANIX_PINENTRY_ZELLIJ") {
    Some(path) => path,
    None => "zellij",
};
const TTY: &str = match option_env!("CANIX_PINENTRY_TTY") {
    Some(path) => path,
    None => "pinentry-tty",
};
const QT: &str = match option_env!("CANIX_PINENTRY_QT") {
    Some(path) => path,
    None => "pinentry-qt",
};
const GPG: Option<&str> = option_env!("CANIX_PINENTRY_GPG");
const LOGINCTL: &str = match option_env!("CANIX_PINENTRY_LOGINCTL") {
    Some(path) => path,
    None => "loginctl",
};
const SYSTEMCTL: &str = match option_env!("CANIX_PINENTRY_SYSTEMCTL") {
    Some(path) => path,
    None => "systemctl",
};
const START_TIMEOUT: Duration = Duration::from_secs(10);
// A peer's greeting follows its SSH connect and, at most, one popup start.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(20);
// `--serve` exits with this before any Assuan output when its seat is not in use.
const ABSENT: u8 = 3;
// Lets a forwarded prompt answer BYE before a closed caller stream stops it.
const CLOSE_GRACE: Duration = Duration::from_secs(1);
const DISCOVERY_RETRY_DELAY: Duration = Duration::from_millis(100);
const LAUNCH_ATTEMPTS: usize = 3;
const DISPLAY_VARIABLES: [&str; 6] = [
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "XDG_SESSION_TYPE",
    "XDG_RUNTIME_DIR",
    "QT_QPA_PLATFORM",
];

// Hex keeps delimiters, whitespace and Assuan escapes out of request metadata.
// Empty fields explicitly clear values inherited from the shared agent.
fn desktop_context(data: &str) -> Option<[Option<OsString>; DISPLAY_VARIABLES.len()]> {
    let fields = data.strip_prefix("canix-pinentry-v1:desktop:")?;
    let values: Option<Vec<_>> = fields
        .split(':')
        .map(|field| {
            if field.is_empty() {
                return Some(None);
            }
            if field.len() % 2 != 0 || !field.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let bytes: Option<Vec<_>> = field
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| {
                    let value = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
                    (value != 0).then_some(value)
                })
                .collect();
            Some(Some(OsString::from_vec(bytes?)))
        })
        .collect();
    let mut values = values?;
    // Older snapshots predate the Qt selector. Clear any stale agent override
    // for them too, letting Qt select the caller's restored display.
    if values.len() == DISPLAY_VARIABLES.len() - 1 {
        values.push(None);
    }
    values.try_into().ok()
}

#[derive(Debug, PartialEq)]
enum Route {
    Desktop,
    Tty,
    /// No display and no terminal: only a configured peer or seat can prompt.
    Headless,
    Zellij {
        pane: u32,
        session: String,
    },
}

fn route(data: Option<&str>) -> Route {
    let Some(data) = data.and_then(|data| data.strip_prefix("canix-pinentry-v1:")) else {
        return Route::Tty;
    };
    if data == "desktop" || desktop_context(&format!("canix-pinentry-v1:{data}")).is_some() {
        return Route::Desktop;
    }
    if data == "headless" {
        return Route::Headless;
    }
    if let Some((pane, session)) = data
        .strip_prefix("zellij:")
        .and_then(|data| data.split_once(':'))
        && let Ok(pane) = pane.parse()
        && !session.is_empty()
        && !session.chars().any(char::is_control)
    {
        return Route::Zellij {
            pane,
            session: session.into(),
        };
    }
    Route::Tty
}

fn client_route(
    data: Option<&str>,
    pane: Option<&str>,
    session: Option<&str>,
    ssh: bool,
    graphical: bool,
    terminal: bool,
) -> Route {
    if let (Some(pane), Some(session)) = (pane, session) {
        return route(Some(&format!("canix-pinentry-v1:zellij:{pane}:{session}")));
    }
    if data.is_some_and(|data| data.starts_with("canix-pinentry-v1:")) {
        let inherited = route(data);
        if inherited != Route::Desktop {
            return inherited;
        }
    }
    if graphical && !ssh {
        Route::Desktop
    } else if terminal {
        Route::Tty
    } else {
        Route::Headless
    }
}

impl Route {
    fn user_data(&self) -> String {
        match self {
            Self::Desktop => {
                let fields = DISPLAY_VARIABLES.map(|name| {
                    env::var_os(name)
                        .unwrap_or_default()
                        .as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()
                });
                format!("canix-pinentry-v1:desktop:{}", fields.join(":"))
            }
            Self::Tty => "canix-pinentry-v1:tty".into(),
            Self::Headless => "canix-pinentry-v1:headless".into(),
            Self::Zellij { pane, session } => {
                format!("canix-pinentry-v1:zellij:{pane}:{session}")
            }
        }
    }
}

fn current_route(inherit_marker: bool) -> Route {
    let data = inherit_marker
        .then(|| env::var("PINENTRY_USER_DATA").ok())
        .flatten();
    let pane = env::var("ZELLIJ_PANE_ID").ok();
    let session = env::var("ZELLIJ_SESSION_NAME").ok();
    let present = |name| env::var_os(name).is_some_and(|value| !value.is_empty());
    client_route(
        data.as_deref(),
        pane.as_deref(),
        session.as_deref(),
        present("SSH_CONNECTION") || present("SSH_TTY"),
        present("DISPLAY") || present("WAYLAND_DISPLAY"),
        // A GPG_TTY inherited from a closed terminal is no origin.
        terminal().is_some()
            || env::var_os("GPG_TTY").is_some_and(|tty| fs::File::open(tty).is_ok())
            || fs::File::open("/dev/tty").is_ok(),
    )
}

// Signing often consumes piped stdin and writes a file. Prefer stdin, then
// interactive stderr/stdout; accept virtual consoles and serial TTYs too.
// Popup metadata retains its stricter owned-PTY check.
fn terminal() -> Option<PathBuf> {
    [
        (0, io::stdin().is_terminal()),
        (2, io::stderr().is_terminal()),
        (1, io::stdout().is_terminal()),
    ]
    .into_iter()
    .find_map(|(fd, terminal)| {
        terminal
            .then(|| fs::read_link(format!("/proc/self/fd/{fd}")).ok())
            .flatten()
    })
}

fn exec_gpg() -> io::Error {
    let Some(gpg) = GPG else {
        return io::Error::other("GPG backend was not configured");
    };
    let mut command = Command::new(gpg);
    command
        .args(env::args_os().skip(1))
        .env("PINENTRY_USER_DATA", current_route(true).user_data());
    if let Some(tty) = terminal() {
        command.env("GPG_TTY", tty);
    }
    command.exec()
}

fn backend(program: &str) -> Command {
    let mut command = Command::new(program);
    command.args(env::args_os().skip(1));
    command
}

fn exec_tty() -> io::Error {
    backend(TTY).exec()
}

fn exec_desktop(agent: bool) -> io::Error {
    let mut command = backend(QT);
    // GnuPG's session options do not carry the complete Wayland context. Restore the
    // complete caller snapshot for agent requests, including absent variables.
    // Legacy markers keep GnuPG's own DISPLAY/XAUTHORITY forwarding; direct age
    // clients always retain their live environment rather than inherited data.
    if agent
        && let Some(context) = env::var("PINENTRY_USER_DATA")
            .ok()
            .as_deref()
            .and_then(desktop_context)
    {
        for (name, value) in DISPLAY_VARIABLES.into_iter().zip(context) {
            if let Some(value) = value {
                command.env(name, value);
            } else {
                command.env_remove(name);
            }
        }
    }
    command.exec()
}

struct RuntimeDirectory(PathBuf);

impl RuntimeDirectory {
    fn create() -> io::Result<Self> {
        let runtime = env::var_os("XDG_RUNTIME_DIR")
            .ok_or_else(|| io::Error::other("XDG_RUNTIME_DIR is unset"))?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let path =
            PathBuf::from(runtime).join(format!("canix-pinentry-{}-{stamp}", std::process::id()));
        DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }
}

impl Drop for RuntimeDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0.join("pane.sock"));
        let _ = fs::remove_file(self.0.join("launch.stderr"));
        let _ = fs::remove_dir(&self.0);
    }
}

fn pane(socket: &Path) -> io::Result<()> {
    let tty = fs::read_link("/proc/self/fd/0")?;
    let mut stream = UnixStream::connect(socket)?;
    writeln!(stream, "{}", tty.display())?;
    // EOF closes the helper after pinentry exits. No PIN is transported here.
    let mut byte = [0];
    let _ = stream.read(&mut byte)?;
    Ok(())
}

fn valid_tty(tty: &str, owner: u32) -> bool {
    tty.strip_prefix("/dev/pts/").is_some_and(|number| {
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
    }) && fs::symlink_metadata(tty)
        .is_ok_and(|metadata| metadata.file_type().is_char_device() && metadata.uid() == owner)
}

struct Popup {
    _directory: RuntimeDirectory,
    _connection: UnixStream,
    tty: String,
}

fn popup(pane: u32, session: &str) -> io::Result<Popup> {
    let directory = RuntimeDirectory::create()?;
    let socket = directory.0.join("pane.sock");
    let listener = UnixListener::bind(&socket)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + START_TIMEOUT;
    // A private file avoids a full stderr pipe blocking the bounded launch wait.
    let mut diagnostics = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.0.join("launch.stderr"))?;
    // https://zellij.dev/documentation/cli-actions#new-pane
    let mut command = Command::new(ZELLIJ);
    command
        .args([
            "--session",
            session,
            "run",
            "--floating",
            "--close-on-exit",
            "--near-current-pane",
            "--name",
            "Hardware key PIN",
            "--pinned",
            "true",
            "--width",
            "70%",
            "--height",
            "12",
            "--",
        ])
        .arg(env::current_exe()?)
        .arg("--pane")
        .arg(&socket)
        .env("ZELLIJ_PANE_ID", pane.to_string())
        .env("ZELLIJ_SESSION_NAME", session)
        .stdin(Stdio::null())
        // Zellij's created pane ID must not enter Assuan stdout.
        .stdout(Stdio::null());
    for attempt in 1..=LAUNCH_ATTEMPTS {
        diagnostics.set_len(0)?;
        diagnostics.rewind()?;
        let mut child = command.stderr(diagnostics.try_clone()?).spawn()?;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Zellij launch timed out",
                ));
            }
            thread::sleep(Duration::from_millis(20));
        };
        diagnostics.rewind()?;
        let mut message = Vec::new();
        (&mut diagnostics).take(4096).read_to_end(&mut message)?;
        if !status.success() {
            // Zellij 0.45.1 src/commands.rs:send_action_to_session exits here
            // before attaching/dispatching when its session IPC probe fails.
            // Do not replay unknown failures, successful dispatches or helpers
            // already connected, and retain one deadline for every attempt.
            if status.code() == Some(1)
                && message == b"There is no active session!\n"
                && attempt < LAUNCH_ATTEMPTS
                && Instant::now() + DISCOVERY_RETRY_DELAY < deadline
                && matches!(listener.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock)
            {
                thread::sleep(DISCOVERY_RETRY_DELAY);
                continue;
            }
            io::stderr().write_all(&message)?;
            return Err(io::Error::other("Zellij could not open the PIN pane"));
        }
        io::stderr().write_all(&message)?;
        break;
    }
    let connection = loop {
        match listener.accept() {
            Ok((connection, _)) => break connection,
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error),
        }
    };
    connection.set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())))?;
    let mut tty = String::new();
    BufReader::new((&connection).take(256)).read_line(&mut tty)?;
    let tty = tty.trim_end().to_string();
    if !valid_tty(&tty, fs::metadata(&directory.0)?.uid()) {
        return Err(io::Error::other("PIN pane did not supply an owned PTY"));
    }
    Ok(Popup {
        _directory: directory,
        _connection: connection,
        tty,
    })
}

/// Rewrites `OPTION ttyname` to the prompt's terminal. A forwarded request's
/// terminal, display and touch file name the requesting host, so they become
/// `NOP`, which keeps Assuan's one-reply-per-command pairing intact.
fn request_line(line: &[u8], tty: Option<&str>, forwarded: bool) -> Vec<u8> {
    if let Ok(text) = std::str::from_utf8(line) {
        let mut words = text.split_whitespace();
        if words
            .next()
            .is_some_and(|command| command.eq_ignore_ascii_case("OPTION"))
            && let Some(option) = words.next()
        {
            let option = option.trim_start_matches("--").split('=').next();
            match (option, tty) {
                (Some("ttyname"), Some(tty)) => {
                    return format!("OPTION ttyname={tty}\n").into_bytes();
                }
                // The external cache would be this seat's keyring, not the requester's.
                (
                    Some(
                        "ttyname"
                        | "display"
                        | "xauthority"
                        | "touch-file"
                        | "parent-wid"
                        | "allow-external-password-cache",
                    ),
                    _,
                ) if forwarded => {
                    return b"NOP\n".to_vec();
                }
                _ => {}
            }
        }
    }
    line.to_vec()
}

fn relay(mut command: Command, tty: Option<String>, forwarded: bool) -> io::Result<ExitCode> {
    // gpg-agent cancels by signalling only this process. A backend left behind
    // keeps Assuan stdout open and its dialog up, blocking the next prompt.
    // SAFETY: prctl(2) is async-signal-safe and touches only the new child.
    unsafe {
        command.pre_exec(|| {
            const PR_SET_PDEATHSIG: i32 = 1;
            const SIGKILL: u64 = 9;
            prctl(PR_SET_PDEATHSIG, SIGKILL);
            Ok(())
        });
    }
    let mut child = command.stdin(Stdio::piped()).spawn()?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("pinentry has no stdin"))?;
    let (closed, caller_gone) = mpsc::channel();
    thread::spawn(move || {
        let mut source = io::stdin().lock();
        let mut line = Vec::new();
        loop {
            line.clear();
            match source.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if input
                        .write_all(&request_line(&line, tty.as_deref(), forwarded))
                        .is_err()
                        || input.flush().is_err()
                    {
                        break;
                    }
                }
            }
        }
        let _ = closed.send(Instant::now());
    });
    // A seat prompt's caller holds the stream for the whole conversation. One
    // that disconnects (agent timeout, or a forwarder that lost its link)
    // cannot reach this backend, and a dialog blocked in GETPIN would outlive
    // it. Origin popups keep their existing behaviour.
    let mut gone = None;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if forwarded {
            gone = gone.or_else(|| caller_gone.try_recv().ok());
            if gone.is_some_and(|at| at.elapsed() >= CLOSE_GRACE) {
                let _ = child.kill();
            }
        }
        thread::sleep(Duration::from_millis(20));
    };
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn run_popup(popup: Popup, mut pinentry: Command, forwarded: bool) -> io::Result<ExitCode> {
    pinentry.args(["--ttyname", &popup.tty]);
    let status = relay(pinentry, Some(popup.tty.clone()), forwarded);
    drop(popup);
    status
}

fn output(program: &str, args: &[&str]) -> io::Result<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!("{program} {args:?} failed")));
    }
    String::from_utf8(output.stdout).map_err(io::Error::other)
}

/// A seat is in use while its graphical session is local, active and unlocked.
/// Lockers must maintain logind's `LockedHint` for an unattended seat to decline.
fn seat_in_use(properties: &str) -> bool {
    let value = |name: &str| {
        properties
            .lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
    };
    value("Active") == Some("yes")
        && value("LockedHint") == Some("no")
        && value("Remote") == Some("no")
        && matches!(value("Type"), Some("wayland" | "x11"))
}

fn present() -> io::Result<bool> {
    let uid = fs::metadata("/proc/self")?.uid().to_string();
    let session = output(LOGINCTL, &["show-user", &uid, "-p", "Display", "--value"])?;
    let session = session.trim();
    if session.is_empty() {
        return Ok(false);
    }
    let properties = output(
        LOGINCTL,
        &[
            "show-session",
            session,
            "-p",
            "Active",
            "-p",
            "LockedHint",
            "-p",
            "Remote",
            "-p",
            "Type",
        ],
    )?;
    Ok(seat_in_use(&properties))
}

/// The focused terminal pane of a client attached to `zellij action list-clients`.
fn client_pane(clients: &str) -> Option<u32> {
    clients.lines().skip(1).find_map(|line| {
        line.split_whitespace()
            .nth(1)?
            .strip_prefix("terminal_")?
            .parse()
            .ok()
    })
}

fn attached_pane() -> Option<(u32, String)> {
    let sessions = output(ZELLIJ, &["list-sessions", "--no-formatting"]).ok()?;
    // Resurrectable sessions have no clients; probing each costs a Zellij call.
    sessions
        .lines()
        .filter(|line| !line.contains("(EXITED"))
        .find_map(|line| {
            // Names may contain spaces; the metadata starts at " [Created".
            let session = line.split(" [Created ").next()?.trim();
            let clients = output(ZELLIJ, &["--session", session, "action", "list-clients"]).ok()?;
            Some((client_pane(&clients)?, session.to_string()))
        })
}

/// Prompts on this seat for a request without a usable origin: a floating pane
/// in an attached Zellij session, else Qt on the user manager's display. The
/// requester's terminal and display options name no surface here either.
fn prompt_here() -> io::Result<ExitCode> {
    if let Some((pane, session)) = attached_pane() {
        match popup(pane, &session) {
            Ok(popup) => return run_popup(popup, Command::new(TTY), true),
            Err(error) => eprintln!("toolbelt-pinentry: {error}; using the desktop"),
        }
    }
    let mut qt = Command::new(QT);
    for name in DISPLAY_VARIABLES {
        qt.env_remove(name);
    }
    for line in output(SYSTEMCTL, &["--user", "show-environment"])?.lines() {
        if let Some((name, value)) = line.split_once('=')
            && DISPLAY_VARIABLES.contains(&name)
            // Shell-quoted values would reach Qt with their quotes; leave them unset.
            && !value.starts_with('\'')
            && !value.starts_with("$'")
        {
            qt.env(name, value);
        }
    }
    relay(qt, None, true)
}

/// Tab-separated peer commands, one per line; `--serve` is appended to each.
fn parse_peers(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .map(|line| {
            line.split('\t')
                .filter(|arg| !arg.is_empty())
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .filter(|peer| !peer.is_empty())
        .collect()
}

fn peers() -> Vec<Vec<String>> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    config
        .and_then(|config| fs::read_to_string(config.join("canix-toolbelt/pinentry-peers")).ok())
        .map(|text| parse_peers(&text))
        .unwrap_or_default()
}

/// This host's name when it needs no Assuan escaping.
fn requester_host() -> Option<String> {
    let host = fs::read_to_string("/proc/sys/kernel/hostname").ok()?;
    let host = host.trim();
    (!host.is_empty()
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._".contains(&byte)))
    .then(|| host.to_string())
}

/// Hosts carry separate cards with the same holder; a PIN typed for the wrong
/// one spends a retry, so a forwarded description names the requesting host.
fn describe_requester(line: &[u8], host: Option<&str>) -> Vec<u8> {
    // Assuan lines hold at most 1000 bytes; a full description stays unlabeled.
    const ASSUAN_LINE: usize = 1000;
    match (line.strip_prefix(b"SETDESC "), host) {
        (Some(description), Some(host)) => {
            let labeled = [
                format!("SETDESC Request from {host}%0A").as_bytes(),
                description,
            ]
            .concat();
            if labeled.len() <= ASSUAN_LINE {
                labeled
            } else {
                line.to_vec()
            }
        }
        _ => line.to_vec(),
    }
}

/// An Assuan greeting is the `OK` status token, alone or followed by text.
fn accepted(greeting: &[u8]) -> bool {
    greeting == b"OK\n" || (greeting.starts_with(b"OK ") && greeting.ends_with(b"\n"))
}

unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
    fn prctl(option: i32, ...) -> i32;
}

fn stop_group(child: &mut std::process::Child) {
    const SIGKILL: i32 = 9;
    // SAFETY: kill(2) takes plain integers; the group is this attempt's own.
    if let Ok(group) = i32::try_from(child.id()) {
        unsafe { kill(-group, SIGKILL) };
    }
    let _ = child.wait();
}

/// Hands the Assuan stream to the first peer whose seat is in use. A peer
/// declines by exiting before its greeting, so no request byte has been read
/// until one accepts; after that its outcome is final.
fn forward(peers: &[Vec<String>]) -> io::Result<Option<ExitCode>> {
    for peer in peers {
        let mut child = match Command::new(&peer[0])
            .args(&peer[1..])
            .arg("--serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Its own group lets a rejected attempt stop the SSH client too.
            .process_group(0)
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                eprintln!("toolbelt-pinentry: {}: {error}", peer[0]);
                continue;
            }
        };
        let stdout = child.stdout.take().expect("piped stdout");
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut greeting = Vec::new();
            let read = reader.read_until(b'\n', &mut greeting);
            let _ = sender.send(read.map(|_| (greeting, reader)));
        });
        // Anything but a complete pinentry greeting (EOF, a login banner, ERR)
        // declines; stop the whole attempt so no transport outlives it.
        let greeting = match receiver.recv_timeout(FORWARD_TIMEOUT) {
            Ok(Ok((greeting, reader))) if accepted(&greeting) => Some((greeting, reader)),
            _ => None,
        };
        let Some((greeting, mut reader)) = greeting else {
            stop_group(&mut child);
            continue;
        };
        let mut input = child.stdin.take().expect("piped stdin");
        let host = requester_host();
        thread::spawn(move || {
            let mut source = io::stdin().lock();
            let mut line = Vec::new();
            while source
                .read_until(b'\n', &mut line)
                .is_ok_and(|read| read > 0)
            {
                let line_out = describe_requester(&line, host.as_deref());
                if input.write_all(&line_out).is_err() || input.flush().is_err() {
                    break;
                }
                line.clear();
            }
        });
        let mut stdout = io::stdout().lock();
        stdout.write_all(&greeting)?;
        stdout.flush()?;
        io::copy(&mut reader, &mut stdout)?;
        stdout.flush()?;
        let status = child.wait()?;
        return Ok(Some(ExitCode::from(status.code().unwrap_or(1) as u8)));
    }
    Ok(None)
}

fn run() -> io::Result<ExitCode> {
    let args: Vec<_> = env::args_os().collect();
    let name = args.first().and_then(|arg| Path::new(arg).file_name());
    if name.is_some_and(|name| name == "gpg" || name == "gpg2") {
        return Err(exec_gpg());
    }
    if args.get(1).is_some_and(|arg| arg == "--context") && args.len() == 2 {
        // Shell hooks refresh from live context, not their previous marker.
        // They run interactively; a headless marker would stick to children.
        let route = match current_route(false) {
            Route::Headless => Route::Tty,
            route => route,
        };
        println!("{}", route.user_data());
        return Ok(ExitCode::SUCCESS);
    }
    if args.get(1).is_some_and(|arg| arg == "--serve") && args.len() == 2 {
        // A peer forwarded this request; never forward it again.
        if !present().unwrap_or(false) {
            return Ok(ExitCode::from(ABSENT));
        }
        return prompt_here();
    }
    if args.get(1).is_some_and(|arg| arg == "--pane") {
        if args.len() != 3 {
            return Err(io::Error::other("--pane requires one socket path"));
        }
        pane(Path::new(&args[2]))?;
        return Ok(ExitCode::SUCCESS);
    }
    let agent = name.is_some_and(|name| name == "canix-toolbelt-pinentry-agent");
    let request = if agent {
        // GnuPG forwards this per request, unlike the agent's startup environment.
        route(env::var("PINENTRY_USER_DATA").ok().as_deref())
    } else {
        current_route(true)
    };
    // Peers are opt-in; without them every route behaves as before.
    let peers = match request {
        Route::Desktop | Route::Headless => peers(),
        _ => Vec::new(),
    };
    match request {
        // An unknown local seat state (no logind user) counts as absent; local
        // Qt still follows when no peer accepts.
        Route::Desktop if !peers.is_empty() && !present().unwrap_or(false) => {
            match forward(&peers)? {
                Some(status) => Ok(status),
                None => Err(exec_desktop(agent)),
            }
        }
        Route::Desktop => Err(exec_desktop(agent)),
        Route::Headless if !peers.is_empty() => {
            if present().unwrap_or(false) {
                return prompt_here();
            }
            match forward(&peers)? {
                Some(status) => Ok(status),
                None => Err(exec_tty()),
            }
        }
        Route::Tty | Route::Headless => Err(exec_tty()),
        Route::Zellij { pane, session } => match popup(pane, &session) {
            Ok(popup) => run_popup(popup, backend(TTY), false),
            Err(error) => {
                eprintln!("toolbelt-pinentry: {error}; using the requesting terminal");
                // No protocol bytes have been consumed. Only startup failure can
                // fall back; cancellation/timeout of an active prompt ends it.
                Err(exec_tty())
            }
        },
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("toolbelt-pinentry: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
