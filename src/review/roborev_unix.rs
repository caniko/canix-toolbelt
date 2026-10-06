//! Bounded local-only v0.71.0 daemon API. No implicit daemon startup or TCP.
use super::{Error, RoborevDispatch, RoborevRunner};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    },
    path::PathBuf,
    time::{Duration, Instant},
};

const MAX_RESPONSE: usize = 16 * 1024 * 1024;

/// Decode one complete connection-close HTTP response. A JSON document alone
/// cannot prove that a chunk terminator or its required trailers were complete.
pub fn decode_roborev_http(response: &[u8]) -> Result<Value, Error> {
    let invalid = || Error("incomplete or invalid local daemon HTTP framing".into());
    if response.len() > MAX_RESPONSE {
        return Err(Error("daemon response exceeded size limit".into()));
    }
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(invalid)?;
    let headers = std::str::from_utf8(&response[..split]).map_err(|_| invalid())?;
    let mut lines = headers.split("\r\n");
    let status: Vec<_> = lines
        .next()
        .ok_or_else(invalid)?
        .split_whitespace()
        .collect();
    if status.len() < 2 || !matches!(status[0], "HTTP/1.0" | "HTTP/1.1") {
        return Err(invalid());
    }
    if !matches!(status[1], "200" | "201") {
        return Err(Error(
            "roborev local API returned a non-success status; response body omitted".into(),
        ));
    }
    let mut length = None;
    let mut chunked = false;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or_else(invalid)?;
        if !valid_header_name(name) {
            return Err(invalid());
        }
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(invalid());
            }
            length = Some(value.trim().parse::<usize>().map_err(|_| invalid())?);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            if chunked || !value.trim().eq_ignore_ascii_case("chunked") {
                return Err(invalid());
            }
            chunked = true;
        }
    }
    let raw = &response[split + 4..];
    let body = if chunked {
        if length.is_some() {
            return Err(invalid());
        }
        let mut decoded = Vec::new();
        let mut cursor = 0;
        loop {
            let end = raw[cursor..]
                .windows(2)
                .position(|w| w == b"\r\n")
                .ok_or_else(invalid)?
                + cursor;
            let size = usize::from_str_radix(
                std::str::from_utf8(&raw[cursor..end])
                    .map_err(|_| invalid())?
                    .split(';')
                    .next()
                    .ok_or_else(invalid)?,
                16,
            )
            .map_err(|_| invalid())?;
            cursor = end + 2;
            if size == 0 {
                loop {
                    let end = raw[cursor..]
                        .windows(2)
                        .position(|w| w == b"\r\n")
                        .ok_or_else(invalid)?
                        + cursor;
                    let trailer = &raw[cursor..end];
                    cursor = end + 2;
                    if trailer.is_empty() {
                        break;
                    }
                    let (name, _) = std::str::from_utf8(trailer)
                        .map_err(|_| invalid())?
                        .split_once(':')
                        .ok_or_else(invalid)?;
                    if !valid_header_name(name)
                        || name.eq_ignore_ascii_case("content-length")
                        || name.eq_ignore_ascii_case("transfer-encoding")
                    {
                        return Err(invalid());
                    }
                }
                if cursor != raw.len() {
                    return Err(invalid());
                }
                break;
            }
            let end = cursor.checked_add(size).ok_or_else(invalid)?;
            if end.checked_add(2).is_none_or(|n| n > raw.len()) || raw[end..end + 2] != *b"\r\n" {
                return Err(invalid());
            }
            decoded.extend_from_slice(&raw[cursor..end]);
            cursor = end + 2;
        }
        decoded
    } else {
        if length.is_some_and(|length| length != raw.len()) {
            return Err(invalid());
        }
        raw.to_vec()
    };
    serde_json::from_slice(&body).map_err(|_| Error("invalid roborev local API document".into()))
}

fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

fn remaining(started: Instant, timeout: Duration) -> Result<Duration, Error> {
    timeout
        .checked_sub(started.elapsed())
        .filter(|value| !value.is_zero())
        .ok_or_else(|| {
            Error("roborev local API deadline exceeded; mutation outcome may be unknown".into())
        })
}

/// Native local-only daemon transport. The caller owns checkout verification,
/// worker qualification and mutation authorization; connecting grants none of them.
pub struct RoborevHttp {
    socket: PathBuf,
    timeout: Duration,
}

impl RoborevHttp {
    /// Select an absolute private daemon socket and a per-request deadline <=60s.
    /// Connecting to a missing daemon fails; it never starts an unmanaged daemon.
    pub fn new(socket: PathBuf, timeout: Duration) -> Result<Self, Error> {
        if !socket.is_absolute() || timeout.is_zero() || timeout > Duration::from_secs(60) {
            return Err(Error(
                "an absolute roborev socket and bounded positive timeout are required".into(),
            ));
        }
        Ok(Self { socket, timeout })
    }
    /// Bounded GET or POST to the explicitly selected daemon. No redirects,
    /// proxies, automatic mutation retries, TCP fallback or daemon autostart.
    pub fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, Error> {
        if !path.starts_with("/api/")
            || path.chars().any(|c| c.is_control() || c.is_whitespace())
            || path.contains('#')
            || !matches!((method, body.is_some()), ("GET", false) | ("POST", true))
        {
            return Err(Error("invalid local daemon method or API path".into()));
        }
        let started = Instant::now();
        let metadata = fs::symlink_metadata(&self.socket)?;
        if !metadata.file_type().is_socket() || metadata.mode() & 0o077 != 0 {
            return Err(Error(
                "daemon socket must be private and nonsymlinked".into(),
            ));
        }
        // Nonblocking connect fails closed on a saturated backlog rather than
        // allowing connect time to exceed the whole-request deadline.
        let descriptor = rustix::net::socket_with(
            rustix::net::AddressFamily::UNIX,
            rustix::net::SocketType::STREAM,
            rustix::net::SocketFlags::NONBLOCK | rustix::net::SocketFlags::CLOEXEC,
            None,
        )
        .map_err(std::io::Error::from)?;
        let address =
            rustix::net::SocketAddrUnix::new(&self.socket).map_err(std::io::Error::from)?;
        rustix::net::connect(&descriptor, &address).map_err(std::io::Error::from)?;
        rustix::fs::fcntl_setfl(&descriptor, rustix::fs::OFlags::empty())
            .map_err(std::io::Error::from)?;
        let mut stream = UnixStream::from(descriptor);
        let body = body
            .map(serde_json::to_vec)
            .transpose()?
            .unwrap_or_default();
        let header = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let failed =
            |_| Error("roborev local API request failed; mutation outcome may be unknown".into());
        for mut bytes in [header.as_bytes(), body.as_slice()] {
            while !bytes.is_empty() {
                stream.set_write_timeout(Some(remaining(started, self.timeout)?))?;
                let count = stream.write(bytes).map_err(failed)?;
                if count == 0 {
                    return Err(failed(std::io::Error::from(std::io::ErrorKind::WriteZero)));
                }
                bytes = &bytes[count..];
            }
        }
        let mut response = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            stream.set_read_timeout(Some(remaining(started, self.timeout)?))?;
            let count = stream.read(&mut buffer).map_err(failed)?;
            if count == 0 {
                break;
            }
            if response.len() + count > MAX_RESPONSE {
                return Err(Error("daemon response exceeded size limit".into()));
            }
            response.extend_from_slice(&buffer[..count]);
        }
        decode_roborev_http(&response)
    }
    /// Collect every job for a checkout, including panel/classification jobs and
    /// terminal failures. Incomplete, cyclic or duplicate pages fail closed.
    pub fn jobs(&self, checkout: &str) -> Result<Vec<Value>, Error> {
        let mut jobs = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for _ in 0..100 {
            let mut url = url::Url::parse("http://127.0.0.1/api/jobs")
                .map_err(|_| Error("invalid local jobs endpoint".into()))?;
            url.query_pairs_mut()
                .append_pair("repo", checkout)
                .append_pair("limit", "100")
                .append_pair("include_panel_members", "true")
                .append_pair("hide_classify_jobs", "false");
            if let Some(cursor) = &cursor {
                url.query_pairs_mut().append_pair("cursor", cursor);
            }
            let page = self.request(
                "GET",
                &format!("{}?{}", url.path(), url.query().unwrap_or_default()),
                None,
            )?;
            let rows = page["jobs"]
                .as_array()
                .ok_or_else(|| Error("missing complete roborev jobs page".into()))?;
            for row in rows {
                let id = row["id"]
                    .as_u64()
                    .filter(|id| *id > 0)
                    .ok_or_else(|| Error("missing persisted job ID".into()))?;
                if !ids.insert(id) {
                    return Err(Error(
                        "duplicate roborev job across paginated observations".into(),
                    ));
                }
                jobs.push(row.clone());
            }
            match page["has_more"].as_bool() {
                Some(false) => return Ok(jobs),
                Some(true) => {
                    let next = page["next_cursor"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| Error("incomplete roborev jobs pagination".into()))?;
                    if !seen_cursors.insert(next.to_owned()) {
                        return Err(Error("cyclic roborev jobs pagination".into()));
                    }
                    cursor = Some(next.to_owned());
                }
                None => return Err(Error("missing roborev pagination completeness flag".into())),
            }
        }
        Err(Error(
            "roborev jobs collection exceeds its complete-page bound".into(),
        ))
    }
}

/// Exact PR-comparison runner with mandatory consumer checkout/worker verification.
pub struct RoborevUnix<V> {
    api: RoborevHttp,
    verify: V,
}

impl<V: FnMut(&RoborevDispatch) -> Result<(), Error>> RoborevUnix<V> {
    /// Select a private local daemon and the controller's trusted verifier.
    pub fn new(socket: PathBuf, timeout: Duration, verify: V) -> Result<Self, Error> {
        Ok(Self {
            api: RoborevHttp::new(socket, timeout)?,
            verify,
        })
    }
}
impl<V: FnMut(&RoborevDispatch) -> Result<(), Error>> RoborevRunner for RoborevUnix<V> {
    fn verify_checkout(&mut self, dispatch: &RoborevDispatch) -> Result<(), Error> {
        (self.verify)(dispatch)
    }
    fn jobs(&mut self, dispatch: &RoborevDispatch) -> Result<Vec<Value>, Error> {
        self.api.jobs(&dispatch.checkout)
    }
    fn enqueue(&mut self, dispatch: &RoborevDispatch) -> Result<u64, Error> {
        let response = self.api.request("POST", "/api/enqueue", Some(&json!({
            "repo_path":dispatch.checkout,"git_ref":format!("{}..{}",dispatch.intent.candidate.base,dispatch.intent.candidate.head),
            "agent":dispatch.agent,"agentic":false,"panel":"none","review_type":"default","min_severity":"low",
        })))?;
        response["id"]
            .as_u64()
            .filter(|id| *id > 0)
            .ok_or_else(|| Error("roborev enqueue did not produce one persisted job".into()))
    }
    fn saved_review(&mut self, job_id: u64) -> Result<Value, Error> {
        if job_id == 0 {
            return Err(Error("positive persisted roborev job ID required".into()));
        }
        self.api
            .request("GET", &format!("/api/review?job_id={job_id}"), None)
    }
}
