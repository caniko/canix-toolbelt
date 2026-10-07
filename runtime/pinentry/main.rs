//! Request-local pinentry routing. PIN responses go straight to the caller on
//! Assuan stdout; the private Zellij socket transports only terminal metadata.

use std::env;
use std::fs::{self, DirBuilder};
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
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
const START_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, PartialEq)]
enum Route {
    Desktop,
    Tty,
    Zellij { pane: u32, session: String },
}

fn route(data: Option<&str>) -> Route {
    let Some(data) = data.and_then(|data| data.strip_prefix("canix-pinentry-v1:")) else {
        return Route::Tty;
    };
    if data == "desktop" {
        return Route::Desktop;
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
    } else {
        Route::Tty
    }
}

impl Route {
    fn user_data(&self) -> String {
        match self {
            Self::Desktop => "canix-pinentry-v1:desktop".into(),
            Self::Tty => "canix-pinentry-v1:tty".into(),
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
    )
}

fn exec_gpg() -> io::Error {
    let Some(gpg) = GPG else {
        return io::Error::other("GPG backend was not configured");
    };
    let mut command = Command::new(gpg);
    command
        .args(env::args_os().skip(1))
        .env("PINENTRY_USER_DATA", current_route(true).user_data());
    // stdin is already open in the requesting process: accept virtual consoles
    // and serial terminals too. Popup metadata retains its stricter PTY check.
    if io::stdin().is_terminal()
        && let Ok(tty) = fs::read_link("/proc/self/fd/0")
    {
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

fn exec_desktop() -> io::Error {
    // GnuPG supplies the requesting session's display per call. A manager-wide
    // environment can belong to a different graphical login or a stale session.
    backend(QT).exec()
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
    // https://zellij.dev/documentation/cli-actions#new-pane
    let mut child = Command::new(ZELLIJ)
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
        .stdout(Stdio::null())
        .spawn()?;
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                return Err(io::Error::other("Zellij could not open the PIN pane"));
            }
            break;
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

fn request_line(line: &[u8], tty: &str) -> Vec<u8> {
    if let Ok(text) = std::str::from_utf8(line) {
        let mut words = text.split_whitespace();
        if words
            .next()
            .is_some_and(|command| command.eq_ignore_ascii_case("OPTION"))
            && words.next().is_some_and(|option| {
                option.trim_start_matches("--").split('=').next() == Some("ttyname")
            })
        {
            return format!("OPTION ttyname={tty}\n").into_bytes();
        }
    }
    line.to_vec()
}

fn run_popup(popup: Popup) -> io::Result<ExitCode> {
    let mut child = backend(TTY)
        .args(["--ttyname", &popup.tty])
        .stdin(Stdio::piped())
        .spawn()?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("pinentry has no stdin"))?;
    let tty = popup.tty.clone();
    thread::spawn(move || {
        let mut source = io::stdin().lock();
        let mut line = Vec::new();
        loop {
            line.clear();
            match source.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if input.write_all(&request_line(&line, &tty)).is_err()
                        || input.flush().is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
    let status = child.wait()?;
    drop(popup);
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn run() -> io::Result<ExitCode> {
    let args: Vec<_> = env::args_os().collect();
    let name = args.first().and_then(|arg| Path::new(arg).file_name());
    if name.is_some_and(|name| name == "gpg" || name == "gpg2") {
        return Err(exec_gpg());
    }
    if args.get(1).is_some_and(|arg| arg == "--context") && args.len() == 2 {
        // Shell hooks refresh from live context, not their previous marker.
        println!("{}", current_route(false).user_data());
        return Ok(ExitCode::SUCCESS);
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
    match request {
        Route::Desktop => Err(exec_desktop()),
        Route::Tty => Err(exec_tty()),
        Route::Zellij { pane, session } => match popup(pane, &session) {
            Ok(popup) => run_popup(popup),
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
