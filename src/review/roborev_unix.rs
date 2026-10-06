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
    time::Duration,
};
use ureq::unversioned::{
    resolver::DefaultResolver,
    transport::{Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport},
};

#[derive(Debug)]
struct UnixConnector(PathBuf);

impl<In: Transport> Connector<In> for UnixConnector {
    type Out = UnixTransport;
    fn connect(
        &self,
        details: &ConnectionDetails,
        _: Option<In>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        // A literal loopback authority is only an HTTP framing label. This
        // connector has no TCP fallback, proxy or redirect path.
        if details.uri.scheme_str() != Some("http")
            || details.uri.authority().map(|a| a.as_str()) != Some("127.0.0.1")
        {
            return Err(std::io::Error::other("unsupported local daemon authority").into());
        }
        let metadata = fs::symlink_metadata(&self.0)?;
        if !metadata.file_type().is_socket() || metadata.mode() & 0o077 != 0 {
            return Err(
                std::io::Error::other("daemon socket must be private and nonsymlinked").into(),
            );
        }
        // Nonblocking Unix connect either succeeds immediately or fails closed,
        // including a saturated backlog. It cannot exceed the request deadline.
        let descriptor = rustix::net::socket_with(
            rustix::net::AddressFamily::UNIX,
            rustix::net::SocketType::STREAM,
            rustix::net::SocketFlags::NONBLOCK | rustix::net::SocketFlags::CLOEXEC,
            None,
        )
        .map_err(std::io::Error::from)?;
        let address = rustix::net::SocketAddrUnix::new(&self.0).map_err(std::io::Error::from)?;
        rustix::net::connect(&descriptor, &address).map_err(std::io::Error::from)?;
        rustix::fs::fcntl_setfl(&descriptor, rustix::fs::OFlags::empty())
            .map_err(std::io::Error::from)?;
        Ok(Some(UnixTransport {
            stream: UnixStream::from(descriptor),
            buffers: LazyBuffers::new(
                details.config.input_buffer_size(),
                details.config.output_buffer_size(),
            ),
        }))
    }
}

#[derive(Debug)]
struct UnixTransport {
    stream: UnixStream,
    buffers: LazyBuffers,
}

fn io_error(error: std::io::Error, timeout: NextTimeout) -> ureq::Error {
    if matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        ureq::Error::Timeout(timeout.reason)
    } else {
        error.into()
    }
}

impl Transport for UnixTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }
    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.stream
            .set_write_timeout(timeout.not_zero().map(|duration| *duration))?;
        self.stream
            .write_all(&self.buffers.output()[..amount])
            .map_err(|error| io_error(error, timeout))
    }
    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.stream
            .set_read_timeout(timeout.not_zero().map(|duration| *duration))?;
        let amount = self
            .stream
            .read(self.buffers.input_append_buf())
            .map_err(|error| io_error(error, timeout))?;
        self.buffers.input_appended(amount);
        Ok(amount > 0)
    }
    fn is_open(&mut self) -> bool {
        false
    } // Never reuse connections or retry mutations.
}

/// Native local daemon runner with a mandatory consumer checkout/worker verifier.
/// The verifier must check the immutable full commits and trusted configuration
/// before dispatch; this transport is not a filesystem/credential boundary.
pub struct RoborevUnix<V> {
    api: ureq::Agent,
    verify: V,
}

impl<V: FnMut(&RoborevDispatch) -> Result<(), Error>> RoborevUnix<V> {
    /// Select an absolute private daemon socket and a per-request deadline <=60s.
    /// Connecting to a missing daemon fails; it never starts an unmanaged daemon.
    pub fn new(socket: PathBuf, timeout: Duration, verify: V) -> Result<Self, Error> {
        if !socket.is_absolute() || timeout.is_zero() || timeout > Duration::from_secs(60) {
            return Err(Error(
                "an absolute roborev socket and bounded positive timeout are required".into(),
            ));
        }
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .max_redirects(0)
            .proxy(None)
            .max_idle_connections(0)
            .http_status_as_error(false)
            .build();
        Ok(Self {
            api: ureq::Agent::with_parts(config, UnixConnector(socket), DefaultResolver::default()),
            verify,
        })
    }

    fn request(&self, url: &str, body: Option<&Value>) -> Result<Value, Error> {
        let result = if let Some(body) = body {
            self.api
                .post(url)
                .header("Connection", "close")
                .send_json(body)
        } else {
            self.api.get(url).header("Connection", "close").call()
        };
        let mut response = result.map_err(|_| {
            Error("roborev local API request failed; mutation outcome may be unknown".into())
        })?;
        if !response.status().is_success() {
            return Err(Error(format!(
                "roborev local API returned HTTP {}; response body omitted",
                response.status().as_u16()
            )));
        }
        let bytes = response
            .body_mut()
            .with_config()
            .limit(16 * 1024 * 1024)
            .read_to_vec()
            .map_err(|_| {
                Error("roborev response is incomplete or exceeds its complete-content bound".into())
            })?;
        serde_json::from_slice(&bytes)
            .map_err(|_| Error("invalid roborev local API document".into()))
    }
}

impl<V: FnMut(&RoborevDispatch) -> Result<(), Error>> RoborevRunner for RoborevUnix<V> {
    fn verify_checkout(&mut self, dispatch: &RoborevDispatch) -> Result<(), Error> {
        (self.verify)(dispatch)
    }

    fn jobs(&mut self, dispatch: &RoborevDispatch) -> Result<Vec<Value>, Error> {
        let mut jobs = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for _ in 0..100 {
            let mut url = url::Url::parse("http://127.0.0.1/api/jobs")
                .map_err(|_| Error("invalid local jobs endpoint".into()))?;
            url.query_pairs_mut()
                .append_pair("repo", &dispatch.checkout)
                .append_pair("limit", "100")
                .append_pair("include_panel_members", "true")
                .append_pair("hide_classify_jobs", "false");
            if let Some(cursor) = &cursor {
                url.query_pairs_mut().append_pair("cursor", cursor);
            }
            let page = self.request(url.as_str(), None)?;
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

    fn enqueue(&mut self, dispatch: &RoborevDispatch) -> Result<u64, Error> {
        let response = self.request("http://127.0.0.1/api/enqueue", Some(&json!({
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
        self.request(
            &format!("http://127.0.0.1/api/review?job_id={job_id}"),
            None,
        )
    }
}
