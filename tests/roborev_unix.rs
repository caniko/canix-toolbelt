#![cfg(all(feature = "review", unix))]

use canix_toolbelt::review::{RoborevRunner, RoborevUnix};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    sync::Arc,
    time::Duration,
};

fn fixture(
    responses: Vec<Value>,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::thread::JoinHandle<Vec<String>>,
) {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                bytes.push(byte[0]);
                if bytes.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let header = String::from_utf8(bytes).unwrap();
            let length = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(|v| v.parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            requests.push(header + &String::from_utf8(body).unwrap());
            let body = serde_json::to_vec(&response).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        }
        requests
    });
    (directory, socket, server)
}

fn dispatch() -> canix_toolbelt::review::RoborevDispatch {
    use canix_toolbelt::review::{Candidate, Intent, RoborevDispatch};
    RoborevDispatch {
        checkout: "/fixture/exclusive repo".into(),
        agent: "opencode".into(),
        expected_files: 1,
        intent: Intent {
            schema_version: 1,
            id: "request".into(),
            policy_digest: "fixture".into(),
            baseline_review_ids: vec![],
            candidate: Candidate {
                url: "https://github.com/example/project/pull/1".into(),
                source_repository: "example/fork".into(),
                source_branch: "feature".into(),
                target_branch: "main".into(),
                head: "a".repeat(40),
                base: "b".repeat(40),
                draft: false,
                open: true,
            },
        },
    }
}

#[test]
fn unix_api_preserves_exact_range_and_reads_every_page_and_persisted_review() {
    let (directory, socket, server) = fixture(vec![
        json!({"jobs":[{"id":2}],"has_more":true,"next_cursor":"opaque cursor"}),
        json!({"jobs":[{"id":1}],"has_more":false}),
        json!({"id":7,"uuid":"11111111-1111-1111-1111-111111111111"}),
        json!({"id":9,"job_id":7}),
    ]);
    let verified = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = verified.clone();
    let mut runner = RoborevUnix::new(socket, Duration::from_secs(5), move |_: &_| {
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    let selected = dispatch();
    runner.verify_checkout(&selected).unwrap();
    assert_eq!(runner.jobs(&selected).unwrap().len(), 2);
    let enqueued = runner.enqueue(&selected).unwrap();
    assert_eq!(enqueued.id, 7);
    assert_eq!(enqueued.uuid, "11111111-1111-1111-1111-111111111111");
    assert_eq!(runner.saved_review(7).unwrap()["job_id"], 7);
    assert_eq!(verified.load(std::sync::atomic::Ordering::SeqCst), 1);
    let requests = server.join().unwrap();
    assert!(requests[0].contains("repo=%2Ffixture%2Fexclusive+repo"));
    assert!(
        requests[0].contains("include_panel_members=true")
            && requests[0].contains("hide_classify_jobs=false")
    );
    assert!(requests[1].contains("cursor=opaque+cursor"));
    let enqueue: Value =
        serde_json::from_str(requests[2].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        enqueue["git_ref"],
        format!(
            "{}..{}",
            selected.intent.candidate.base, selected.intent.candidate.head
        )
    );
    assert_eq!(enqueue["panel"], "none");
    assert_eq!(enqueue["min_severity"], "low");
    assert_eq!(enqueue["review_type"], "default");
    assert_eq!(enqueue["agentic"], false);
    assert!(
        enqueue.get("custom_prompt").is_none()
            && enqueue.get("model").is_none()
            && enqueue.get("provider").is_none()
    );
    assert!(requests[3].starts_with("GET /api/review?job_id=7 "));
    drop(directory);
}

#[test]
fn cyclic_partial_or_duplicate_pages_cannot_be_complete_evidence() {
    for responses in [
        vec![json!({"jobs":[],"has_more":true})],
        vec![
            json!({"jobs":[{"id":1}],"has_more":true,"next_cursor":"same"}),
            json!({"jobs":[{"id":2}],"has_more":true,"next_cursor":"same"}),
        ],
        vec![json!({"jobs":[{"id":1},{"id":1}],"has_more":false})],
    ] {
        let (_directory, socket, server) = fixture(responses);
        let mut runner = RoborevUnix::new(socket, Duration::from_secs(5), |_: &_| Ok(())).unwrap();
        assert!(runner.jobs(&dispatch()).is_err());
        server.join().unwrap();
    }
}

#[test]
fn missing_public_and_symlinked_socket_cannot_autostart_or_redirect() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("daemon.sock");
    let mut missing =
        RoborevUnix::new(socket.clone(), Duration::from_secs(1), |_: &_| Ok(())).unwrap();
    assert!(missing.jobs(&dispatch()).is_err());
    assert!(!socket.exists());
    let listener = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(missing.jobs(&dispatch()).is_err());
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    let alias = directory.path().join("alias");
    std::os::unix::fs::symlink(&socket, &alias).unwrap();
    let mut redirected = RoborevUnix::new(alias, Duration::from_secs(1), |_: &_| Ok(())).unwrap();
    assert!(redirected.jobs(&dispatch()).is_err());
    drop(listener);
}
