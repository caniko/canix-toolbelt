#![cfg(all(unix, feature = "orchestration"))]
use canix_toolbelt::orchestration::snapshots::*;
use serde_json::{Value, json};
use std::io;

struct Graph(Vec<Value>);
impl GraphQl for Graph {
    fn query(&mut self, _: &str) -> io::Result<Value> {
        if self.0.is_empty() {
            return Err(io::Error::other("unexpected extra query"));
        }
        Ok(self.0.remove(0))
    }
}

fn pull() -> Value {
    json!({"id":"PR_one","state":"OPEN","headRefOid":"head","baseRefOid":"reported","baseRef":{"target":{"oid":"base"}},"baseRefName":"main","updatedAt":"now","isDraft":false,"commits":{"nodes":[]},"comments":{"totalCount":1,"nodes":[{"id":"historical","body":"edited feedback","author":{"__typename":"Bot","login":"chatgpt-codex-connector"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}},"reviews":{"totalCount":0,"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},"reviewThreads":{"totalCount":0,"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}})
}

#[test]
fn complete_history_keeps_old_comment_bodies_and_uses_live_target() {
    let p = pull();
    let mut transport = Graph(vec![
        json!({"repository":{"viewerPermission":"WRITE","pullRequest":p}}),
        json!({"repository":{"pullRequest":p}}),
    ]);
    let got = github(
        &mut transport,
        "https://github.com/owner/repo/pull/1",
        &json!({"unknown":"retained","lastError":"old failure"}),
        "today",
        12,
    )
    .unwrap();
    assert_eq!(got["base"], "base");
    assert_eq!(got["reportedBase"], "reported");
    assert_eq!(got["issueComments"][0]["body"], "edited feedback");
    assert_eq!(got["unknown"], "retained");
    assert!(got.get("lastError").is_none());
}

#[test]
fn truncated_history_and_moving_comparison_are_errors() {
    let mut p = pull();
    p["comments"]["totalCount"] = json!(2);
    let mut transport = Graph(vec![
        json!({"repository":{"viewerPermission":"WRITE","pullRequest":p}}),
    ]);
    assert!(
        github(
            &mut transport,
            "https://github.com/owner/repo/pull/1",
            &json!({}),
            "today",
            12
        )
        .is_err()
    );
    let p = pull();
    let mut moved = p.clone();
    moved["headRefOid"] = json!("next");
    let mut transport = Graph(vec![
        json!({"repository":{"viewerPermission":"WRITE","pullRequest":p}}),
        json!({"repository":{"pullRequest":moved}}),
    ]);
    assert!(
        github(
            &mut transport,
            "https://github.com/owner/repo/pull/1",
            &json!({}),
            "today",
            12
        )
        .is_err()
    );
}

#[test]
fn nested_thread_pagination_retains_outdated_findings_and_detects_cursor_replay() {
    let mut p = pull();
    p["reviewThreads"] = json!({"totalCount":1,"nodes":[{"id":"thread","isResolved":false,"isOutdated":true,"comments":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":"cursor"}}}],"pageInfo":{"hasNextPage":false,"endCursor":null}});
    let next = json!({"node":{"comments":{"nodes":[{"id":"old","body":"outdated finding","author":{"__typename":"Bot","login":"chatgpt-codex-connector"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}});
    let mut transport = Graph(vec![
        json!({"repository":{"viewerPermission":"WRITE","pullRequest":p}}),
        next,
        json!({"repository":{"pullRequest":p}}),
    ]);
    let got = github(
        &mut transport,
        "https://github.com/owner/repo/pull/1",
        &json!({}),
        "today",
        12,
    )
    .unwrap();
    assert_eq!(got["unresolvedCodex"], 1);
    assert_eq!(
        got["threads"][0]["comments"]["nodes"][0]["body"],
        "outdated finding"
    );
    let mut transport = Graph(vec![
        json!({"repository":{"viewerPermission":"WRITE","pullRequest":p}}),
        json!({"node":{"comments":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":"cursor"}}}}),
    ]);
    assert!(
        github(
            &mut transport,
            "https://github.com/owner/repo/pull/1",
            &json!({}),
            "today",
            12
        )
        .is_err()
    );
}

struct Forge {
    calls: Vec<String>,
    inconsistent: bool,
}

impl ForgeRest for Forge {
    fn get(&mut self, host: &str, path: &str) -> io::Result<Value> {
        assert_eq!(host, "codefloe.com");
        self.calls.push(path.into());
        if path == "repos/owner/repo/pulls/1" {
            return Ok(
                json!({"head":{"sha":"abcdef"},"base":{"sha":"123456","ref":"main"},"state":"open","merged":false,"updated_at":"now"}),
            );
        }
        if path.contains("/status?") {
            return Ok(if path.ends_with("page=1") {
                json!({"sha":"abcdef","state":"success","total_count":101,"statuses":(0..100).map(|id| json!({"id":id,"state":"success"})).collect::<Vec<_>>()})
            } else {
                json!({"sha":"abcdef","state":if self.inconsistent { "failure" } else { "success" },"total_count":101,"statuses":[{"id":100,"state":"success"}]})
            });
        }
        Ok(json!([]))
    }
}

#[test]
fn forgejo_collects_all_check_contexts_and_rejects_moving_rollup() {
    let mut transport = Forge {
        calls: vec![],
        inconsistent: false,
    };
    let got = forgejo(
        &mut transport,
        "https://codefloe.com/owner/repo/pulls/1",
        "now",
    )
    .unwrap();
    assert_eq!(got["checks"]["statuses"].as_array().unwrap().len(), 101);
    assert!(
        transport
            .calls
            .iter()
            .any(|path| path.ends_with("status?limit=100&page=2"))
    );
    transport.inconsistent = true;
    assert!(
        forgejo(
            &mut transport,
            "https://codefloe.com/owner/repo/pulls/1",
            "now"
        )
        .is_err()
    );
}
