#![cfg(all(feature = "review", unix))]
use canix_toolbelt::review::{
    Candidate, Intent, Policy, Provider, RoborevHttp, RoborevLocal, RoborevLocalRequest,
    RoborevLocalScope, normalize_roborev_local,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

fn candidate() -> Candidate {
    Candidate {
        url: "https://local.invalid/review/fixture".into(),
        source_repository: "local.invalid/fixture".into(),
        source_branch: "feature".into(),
        target_branch: "trunk".into(),
        head: "a".repeat(40),
        base: "b".repeat(40),
        draft: false,
        open: true,
    }
}
fn policy() -> Policy {
    Policy {
        provider: "roborev".into(),
        transport: "unix-http-v0.71".into(),
        ..Policy::default()
    }
}
fn request(stream: &mut UnixStream) -> (String, Value) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let split = loop {
        let n = stream.read(&mut buffer).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
        if let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break split;
        }
    };
    let headers = std::str::from_utf8(&bytes[..split]).unwrap().to_owned();
    let length: usize = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse().unwrap())
        .unwrap_or(0);
    while bytes.len() < split + 4 + length {
        let n = stream.read(&mut buffer).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
    }
    let body = if length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes[split + 4..]).unwrap()
    };
    (headers.lines().next().unwrap().into(), body)
}

struct Fixture {
    root: tempfile::TempDir,
    job: Arc<Mutex<Option<Value>>>,
    detail: Arc<Mutex<Option<Value>>>,
    enqueues: Arc<Mutex<usize>>,
    stop: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("daemon.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let job = Arc::new(Mutex::new(None::<Value>));
        let detail = Arc::new(Mutex::new(None::<Value>));
        let enqueues = Arc::new(Mutex::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (saved, details, count, stopping) =
            (job.clone(), detail.clone(), enqueues.clone(), stop.clone());
        let server = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                let (line, input) = request(&mut stream);
                let response = if line == "POST /api/enqueue HTTP/1.1" {
                    *count.lock().unwrap() += 1;
                    *saved.lock().unwrap() = Some(
                        json!({"id":7,"uuid":"original-job","repo_path":input["repo_path"],
                        "git_ref":input["git_ref"],"agent":input["agent"],"agentic":false,"min_severity":"low",
                        "status":"queued","diff_content":input["diff_content"]}),
                    );
                    continue; // Persist the enqueue and lose its reply.
                } else if line == "GET /api/jobs?id=7 HTTP/1.1" {
                    details.lock().unwrap().clone().unwrap_or_else(|| json!({"jobs":[saved.lock().unwrap().clone().unwrap()],"has_more":false}))
                } else if line == "GET /api/review?job_id=7 HTTP/1.1" {
                    json!({"id":9,"job_id":7,"job":saved.lock().unwrap().clone().unwrap(),
                        "structured_output":{"schema_version":2,"summary":"No findings","verdict":"pass","findings":[]},
                        "file_coverage":{"reviewed":1,"excluded":0}})
                } else {
                    assert!(line.starts_with("GET /api/jobs?repo="), "{line}");
                    let jobs: Vec<_> = saved
                        .lock()
                        .unwrap()
                        .clone()
                        .into_iter()
                        .map(|mut job| {
                            job.as_object_mut().unwrap().remove("diff_content");
                            job
                        })
                        .collect();
                    json!({"jobs":jobs,"has_more":false})
                };
                let body = serde_json::to_vec(&response).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        Self {
            root,
            job,
            detail,
            enqueues,
            stop,
            server: Some(server),
        }
    }
    fn intent(&self) -> Intent {
        Intent {
            schema_version: 1,
            id: "legacy-request".into(),
            candidate: candidate(),
            policy_digest: policy().digest().unwrap(),
            baseline_review_ids: vec![],
        }
    }
    fn provider(&self) -> RoborevLocal {
        RoborevLocal::new(
            RoborevHttp::new(self.root.path().join("daemon.sock"), Duration::from_secs(2)).unwrap(),
            RoborevLocalRequest {
                snapshot: self.root.path().join("snapshot"),
                git_ref: "dirty".into(),
                agent: "opencode".into(),
                diff: Some("EXACT_CAPTURED_DIFF\n".into()),
                expected_files: 1,
                scope: RoborevLocalScope::WorkingTree,
            },
            self.root.path().join("state"),
        )
        .unwrap()
    }
    fn ledger(&self) -> std::path::PathBuf {
        use sha2::{Digest, Sha256};
        self.root.path().join("state").join(format!(
            "{:x}.json",
            Sha256::digest(self.intent().id.as_bytes())
        ))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.server.take().unwrap().join().unwrap();
    }
}

#[test]
fn shared_http_requires_complete_body_framing() {
    for (response, accepted) in [
        (&b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n"[..], true),
        (&b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n{}"[..], false),
        (&b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n"[..], false),
        (&b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\nnot-a-trailer\r\n\r\n"[..], false),
    ] {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("daemon.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            request(&mut stream);
            stream.write_all(response).unwrap();
        });
        let api = RoborevHttp::new(socket, Duration::from_secs(2)).unwrap();
        assert_eq!(api.request("GET", "/api/fixture", None).is_ok(), accepted,
            "framing accepted incorrectly: {}", String::from_utf8_lossy(response));
        server.join().unwrap();
    }
}

#[test]
fn never_submitted_observation_does_not_claim_an_enqueue_lifetime() {
    let fixture = Fixture::new();
    let evidence = fixture
        .provider()
        .inspect(&candidate(), &policy(), Some(&fixture.intent()))
        .unwrap();
    assert!(evidence.request_receipt.is_none());
    assert!(!fixture.root.path().join("state").exists());
    assert!(fixture.provider().submit(&fixture.intent()).is_err()); // Lost reply, one actual enqueue.
    assert_eq!(*fixture.enqueues.lock().unwrap(), 1);
}

#[test]
fn unknown_local_enqueue_preserves_legacy_identity_and_hydrates_original_diff() {
    let fixture = Fixture::new();
    assert!(fixture.provider().submit(&fixture.intent()).is_err());
    let ledger: Value = serde_json::from_slice(&fs::read(fixture.ledger()).unwrap()).unwrap();
    assert_eq!(ledger["dispatching"], true);
    assert_eq!(ledger["intent"]["id"], "legacy-request");
    for status in ["queued", "running", "done"] {
        fixture.job.lock().unwrap().as_mut().unwrap()["status"] = json!(status);
        let evidence = fixture
            .provider()
            .inspect(&candidate(), &policy(), Some(&fixture.intent()))
            .unwrap();
        assert_eq!(
            evidence.request_receipt.as_deref(),
            Some("roborev:job:7:original-job")
        );
        assert_eq!(evidence.review.is_some(), status == "done");
        if let Some(review) = evidence.review {
            assert_eq!(review.provider_job_id.as_deref(), Some("7"));
            assert_eq!(policy().evaluate(&candidate(), &review).exit_code(), 0);
        }
        assert!(fixture.provider().submit(&fixture.intent()).is_err());
    }
    assert_eq!(*fixture.enqueues.lock().unwrap(), 1);
    fs::remove_file(fixture.ledger()).unwrap();
    assert!(fixture.provider().submit(&fixture.intent()).is_err());
    assert_eq!(*fixture.enqueues.lock().unwrap(), 1);
}

#[test]
fn legacy_local_submission_without_new_anchor_resumes_without_reenqueue() {
    let fixture = Fixture::new();
    assert!(fixture.provider().submit(&fixture.intent()).is_err());
    fs::remove_file(fixture.ledger().with_extension("lock")).unwrap(); // Legacy client had no per-submission anchor.
    assert!(
        fixture
            .provider()
            .inspect(&candidate(), &policy(), Some(&fixture.intent()))
            .unwrap()
            .request_receipt
            .is_some()
    );
    assert!(fixture.provider().submit(&fixture.intent()).is_err());
    assert_eq!(*fixture.enqueues.lock().unwrap(), 1);
}

#[test]
fn untrusted_detail_cannot_change_unknown_ledger_or_reenqueue() {
    for case in [
        "missing",
        "duplicate",
        "id",
        "uuid",
        "diff",
        "absent-diff",
        "range",
        "partial",
        "unknown-completeness",
    ] {
        let fixture = Fixture::new();
        assert!(fixture.provider().submit(&fixture.intent()).is_err());
        let before = fs::read(fixture.ledger()).unwrap();
        let mut job = fixture.job.lock().unwrap().clone().unwrap();
        match case {
            "id" => job["id"] = json!(8),
            "uuid" => job["uuid"] = json!("different-job"),
            "diff" => job["diff_content"] = json!("DIFFERENT_DIFF"),
            "absent-diff" => {
                job.as_object_mut().unwrap().remove("diff_content");
            }
            "range" => job["git_ref"] = json!("different-range"),
            _ => {}
        }
        let jobs = match case {
            "missing" => vec![],
            "duplicate" => vec![job.clone(), job],
            _ => vec![job],
        };
        let mut detail = json!({"jobs":jobs,"has_more":case == "partial"});
        if case == "unknown-completeness" {
            detail.as_object_mut().unwrap().remove("has_more");
        }
        *fixture.detail.lock().unwrap() = Some(detail);
        assert!(
            fixture
                .provider()
                .inspect(&candidate(), &policy(), Some(&fixture.intent()))
                .unwrap_err()
                .to_string()
                .contains("working-tree job")
        );
        assert_eq!(fs::read(fixture.ledger()).unwrap(), before, "{case}");
        assert!(fixture.provider().submit(&fixture.intent()).is_err());
        assert_eq!(*fixture.enqueues.lock().unwrap(), 1);
    }
}

#[test]
fn local_advisory_range_does_not_attest_or_qualify_a_different_pr_base() {
    let job = json!({"id":7,"uuid":"range-job","git_ref":format!("{}..{}", "c".repeat(40), candidate().head)});
    let raw = json!({"id":9,"job_id":7,"job":job,"structured_output":{"schema_version":2,"summary":"No findings","verdict":"pass","findings":[]},
        "file_coverage":{"reviewed":1,"excluded":0}});
    assert!(
        normalize_roborev_local(
            &raw,
            &job,
            &candidate(),
            &policy(),
            1,
            RoborevLocalScope::PullRequest
        )
        .is_err()
    );
    let review = normalize_roborev_local(
        &raw,
        &job,
        &candidate(),
        &policy(),
        1,
        RoborevLocalScope::Committed,
    )
    .unwrap();
    assert_eq!(
        review.reviewed_base.as_deref(),
        Some("c".repeat(40).as_str())
    );
    assert_eq!(policy().evaluate(&candidate(), &review).exit_code(), 0);
    let publication_policy = Policy {
        transport: "github-receipt-v1".into(),
        ..policy()
    };
    assert_ne!(
        publication_policy
            .evaluate(&candidate(), &review)
            .exit_code(),
        0
    );
}

#[test]
fn advisory_comparisons_must_still_attest_the_selected_head() {
    for git_ref in [
        format!("{}..{}", "c".repeat(40), "d".repeat(40)),
        format!("trunk..{}", candidate().head),
        "dirty".into(),
    ] {
        let job = json!({"id":7,"uuid":"range-job","git_ref":git_ref});
        let raw = json!({"id":9,"job_id":7,"job":job,
            "structured_output":{"schema_version":2,"summary":"No findings","verdict":"pass","findings":[]},
            "file_coverage":{"reviewed":1,"excluded":0}});
        assert!(
            normalize_roborev_local(
                &raw,
                &job,
                &candidate(),
                &policy(),
                1,
                RoborevLocalScope::Committed,
            )
            .is_err(),
            "accepted unbound advisory comparison: {git_ref}"
        );
    }
    let job = json!({"id":7,"uuid":"range-job","git_ref":format!("{}..{}", candidate().base, candidate().head)});
    let raw = json!({"id":9,"job_id":7,"job":job,
        "structured_output":{"schema_version":2,"summary":"No findings","verdict":"pass","findings":[]},
        "file_coverage":{"reviewed":1,"excluded":0}});
    assert!(
        normalize_roborev_local(
            &raw,
            &job,
            &candidate(),
            &policy(),
            1,
            RoborevLocalScope::WorkingTree,
        )
        .is_err()
    );
}
