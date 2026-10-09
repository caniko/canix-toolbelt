#![cfg(all(unix, feature = "orchestration"))]
use canix_toolbelt::orchestration::snapshots::*;
use serde_json::{Value, json};
use std::io;

struct Graph(Vec<Value>);
impl GraphQl for Graph {
    fn query(&mut self, query: &str) -> io::Result<Value> {
        if query.contains("comments(first:100") {
            assert!(
                query.contains("totalCount"),
                "nested history must request its count"
            );
        }
        if self.0.is_empty() {
            return Err(io::Error::other("unexpected extra query"));
        }
        Ok(self.0.remove(0))
    }
}

fn pull() -> Value {
    json!({"id":"PR_one","state":"OPEN","headRefOid":"head","baseRefOid":"reported","baseRef":{"target":{"oid":"base"}},"baseRefName":"main","updatedAt":"now","isDraft":false,"commits":{"nodes":[{"commit":{"id":"commit_head","oid":"head","statusCheckRollup":null}}]},"comments":{"totalCount":1,"nodes":[{"id":"historical","body":"edited feedback","author":{"__typename":"Bot","login":"chatgpt-codex-connector"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}},"reviews":{"totalCount":0,"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},"reviewThreads":{"totalCount":0,"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}})
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
    p["reviewThreads"] = json!({"totalCount":1,"nodes":[{"id":"thread","isResolved":false,"isOutdated":true,"comments":{"totalCount":1,"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":"cursor"}}}],"pageInfo":{"hasNextPage":false,"endCursor":null}});
    let next = json!({"node":{"comments":{"totalCount":1,"nodes":[{"id":"old","body":"outdated finding","author":{"__typename":"Bot","login":"chatgpt-codex-connector"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}});
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
        json!({"node":{"comments":{"totalCount":1,"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":"cursor"}}}}),
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

#[test]
fn github_requires_counts_on_every_history_and_nested_connection() {
    for connection in ["comments", "reviews", "reviewThreads", "nestedComments"] {
        for count in [Value::Null, json!("1"), json!(-1)] {
            let mut p = pull();
            if connection == "nestedComments" {
                p["reviewThreads"] = json!({"totalCount":1,"nodes":[{"id":"thread","comments":{"totalCount":count,"nodes":[{"id":"old"}],"pageInfo":{"hasNextPage":false}}}],"pageInfo":{"hasNextPage":false}});
            } else {
                p[connection]["totalCount"] = count;
            }
            let mut transport = Graph(vec![json!({"repository":{"pullRequest":p}})]);
            let error = github(
                &mut transport,
                "https://github.com/owner/repo/pull/1",
                &json!({}),
                "now",
                0,
            )
            .unwrap_err();
            assert!(
                error.to_string().contains("connection total"),
                "{connection}: {error}"
            );
        }
    }
}

#[test]
fn github_requires_unchanged_counts_on_every_following_page() {
    for count in [Value::Null, json!(3), json!("2")] {
        let mut p = pull();
        p["comments"]["totalCount"] = json!(2);
        p["comments"]["pageInfo"] = json!({"hasNextPage":true,"endCursor":"older"});
        let next = json!({"repository":{"pullRequest":{"comments":{"totalCount":count,"nodes":[{"id":"second"}],"pageInfo":{"hasNextPage":false}}}}});
        let mut transport = Graph(vec![json!({"repository":{"pullRequest":p}}), next]);
        let error = github(
            &mut transport,
            "https://github.com/owner/repo/pull/1",
            &json!({}),
            "now",
            0,
        )
        .unwrap_err();
        assert!(error.to_string().contains("connection total changed"));
    }
}

#[test]
fn github_requires_exactly_one_commit_matching_the_checked_head() {
    for commits in [
        json!([]),
        json!([{"commit":{"id":"wrong","oid":"other"}}]),
        json!([{"commit":{"id":"head1","oid":"head"}},{"commit":{"id":"head2","oid":"head"}}]),
    ] {
        let mut p = pull();
        p["commits"]["nodes"] = commits;
        let mut transport = Graph(vec![json!({"repository":{"pullRequest":p}})]);
        let error = github(
            &mut transport,
            "https://github.com/owner/repo/pull/1",
            &json!({}),
            "now",
            0,
        )
        .unwrap_err();
        assert!(error.to_string().contains("commit"));
    }
}

#[test]
fn github_rejects_check_changes_without_head_or_feedback_movement() {
    let mut p = pull();
    p["commits"]["nodes"][0]["commit"]["statusCheckRollup"] = json!({"state":"SUCCESS","contexts":{"totalCount":1,"nodes":[{"id":"check","status":"COMPLETED","conclusion":"SUCCESS"}],"pageInfo":{"hasNextPage":false}}});
    let mut fresh = p.clone();
    fresh["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["nodes"][0]["conclusion"] =
        json!("FAILURE");
    let mut transport = Graph(vec![
        json!({"repository":{"pullRequest":p}}),
        json!({"repository":{"pullRequest":fresh}}),
    ]);
    assert!(
        github(
            &mut transport,
            "https://github.com/owner/repo/pull/1",
            &json!({}),
            "now",
            0
        )
        .is_err()
    );
}

#[test]
fn github_revalidates_check_contexts_beyond_the_initial_page() {
    let mut p = pull();
    let first: Vec<_> = (0..100)
        .map(|id| json!({"id":format!("check_{id}"),"conclusion":"SUCCESS"}))
        .collect();
    p["commits"]["nodes"][0]["commit"]["statusCheckRollup"] = json!({"state":"SUCCESS","contexts":{"totalCount":101,"nodes":first,"pageInfo":{"hasNextPage":true,"endCursor":"last"}}});
    let last = json!({"node":{"statusCheckRollup":{"contexts":{"totalCount":101,"nodes":[{"id":"check_last","conclusion":"SUCCESS"}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
    let mut changed = last.clone();
    changed["node"]["statusCheckRollup"]["contexts"]["nodes"][0]["conclusion"] = json!("FAILURE");
    for final_page in [last.clone(), changed] {
        let should_pass = final_page == last;
        let mut transport = Graph(vec![
            json!({"repository":{"pullRequest":p}}),
            last.clone(),
            json!({"repository":{"pullRequest":p}}),
            final_page,
        ]);
        let result = github(
            &mut transport,
            "https://github.com/owner/repo/pull/1",
            &json!({}),
            "now",
            0,
        );
        assert_eq!(result.is_ok(), should_pass);
        if let Ok(snapshot) = result {
            assert_eq!(
                snapshot["checks"]["contexts"]["nodes"]
                    .as_array()
                    .unwrap()
                    .len(),
                101
            );
        }
        assert!(transport.0.is_empty());
    }
}

struct ForgeScript(Vec<(String, Value)>);
impl ForgeRest for ForgeScript {
    fn get(&mut self, host: &str, path: &str) -> io::Result<Value> {
        assert_eq!(host, "codefloe.com");
        if self.0.is_empty() {
            return Err(io::Error::other("unexpected extra forge request"));
        }
        let (expected, value) = self.0.remove(0);
        assert_eq!(path, expected);
        Ok(value)
    }
}

fn forge_pull() -> Value {
    json!({"head":{"sha":"abcdef"},"base":{"sha":"123456","ref":"main"},"state":"open","merged":false,"updated_at":"now"})
}

fn forge_status() -> Value {
    json!({"sha":"abcdef","state":"success","total_count":1,"statuses":[{"id":1,"state":"success"}]})
}

fn forge_review(review: Value, inline: Value) -> ForgeScript {
    ForgeScript(vec![
        ("repos/owner/repo/pulls/1".into(), forge_pull()),
        (
            "repos/owner/repo/pulls/1/reviews?limit=100&page=1".into(),
            json!([review]),
        ),
        (
            "repos/owner/repo/issues/1/comments?limit=100&page=1".into(),
            json!([]),
        ),
        (
            "repos/owner/repo/pulls/1/reviews/1/comments?limit=100&page=1".into(),
            inline,
        ),
        (
            "repos/owner/repo/commits/abcdef/status?limit=100&page=1".into(),
            forge_status(),
        ),
        ("repos/owner/repo/pulls/1".into(), forge_pull()),
        (
            "repos/owner/repo/commits/abcdef/status?limit=100&page=1".into(),
            forge_status(),
        ),
    ])
}

#[test]
fn forgejo_requires_declared_complete_inline_comment_counts() {
    for count in [Value::Null, json!("1"), json!(-1), json!(2001)] {
        let mut transport = forge_review(json!({"id":1,"comments_count":count}), json!([]));
        let error = forgejo(
            &mut transport,
            "https://codefloe.com/owner/repo/pulls/1",
            "now",
        )
        .unwrap_err();
        assert!(error.to_string().contains("review comment count"));
    }
    for inline in [json!([]), json!([{"id":10}]), json!([{"id":10},{"id":11}])] {
        let should_pass = inline.as_array().unwrap().len() == 2;
        let mut transport = forge_review(json!({"id":1,"comments_count":2}), inline);
        let result = forgejo(
            &mut transport,
            "https://codefloe.com/owner/repo/pulls/1",
            "now",
        );
        assert_eq!(result.is_ok(), should_pass);
        if let Ok(snapshot) = result {
            assert_eq!(snapshot["inlineComments"].as_array().unwrap().len(), 2);
            assert!(transport.0.is_empty());
        }
    }
}

#[test]
fn forgejo_rejects_missing_and_replayed_history_identities() {
    let first: Vec<_> = (1..=100)
        .map(|id| json!({"id":id,"comments_count":0}))
        .collect();
    let mut replay = ForgeScript(vec![
        ("repos/owner/repo/pulls/1".into(), forge_pull()),
        (
            "repos/owner/repo/pulls/1/reviews?limit=100&page=1".into(),
            json!(first),
        ),
        (
            "repos/owner/repo/pulls/1/reviews?limit=100&page=2".into(),
            json!([{"id":100,"comments_count":0}]),
        ),
    ]);
    assert!(
        forgejo(
            &mut replay,
            "https://codefloe.com/owner/repo/pulls/1",
            "now"
        )
        .unwrap_err()
        .to_string()
        .contains("history identity")
    );
    for row in [
        json!({"comments_count":0}),
        json!({"id":null,"comments_count":0}),
        json!({"id":"1","comments_count":0}),
    ] {
        let mut transport = forge_review(row, json!([]));
        assert!(
            forgejo(
                &mut transport,
                "https://codefloe.com/owner/repo/pulls/1",
                "now"
            )
            .unwrap_err()
            .to_string()
            .contains("history identity")
        );
    }
    let mut repeated_inline = forge_review(
        json!({"id":1,"comments_count":2}),
        json!([{"id":10},{"id":10}]),
    );
    assert!(
        forgejo(
            &mut repeated_inline,
            "https://codefloe.com/owner/repo/pulls/1",
            "now"
        )
        .unwrap_err()
        .to_string()
        .contains("history identity")
    );
}

#[test]
fn forgejo_rechecks_single_page_ci_after_the_final_pull_observation() {
    let mut transport = forge_review(json!({"id":1,"comments_count":1}), json!([{"id":10}]));
    let mut moved = forge_status();
    moved["state"] = json!("failure");
    moved["statuses"][0]["state"] = json!("failure");
    transport.0.last_mut().unwrap().1 = moved;
    let error = forgejo(
        &mut transport,
        "https://codefloe.com/owner/repo/pulls/1",
        "now",
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("check rollup moved after history")
    );
    assert!(transport.0.is_empty());
}

#[test]
fn dot_segment_coordinates_are_rejected_before_transport() {
    for url in [
        "https://codefloe.com/../repo/pulls/1",
        "https://codefloe.com/owner/./pulls/1",
        "https://codefloe.com/owner/../pulls/1",
        "https://github.com/./repo/pull/1",
        "https://github.com/owner/../pull/1",
    ] {
        if url.contains("github.com") {
            let mut transport = Graph(vec![]);
            assert!(
                github(&mut transport, url, &json!({}), "now", 0)
                    .unwrap_err()
                    .to_string()
                    .contains("coordinate")
            );
        } else {
            let mut transport = ForgeScript(vec![]);
            assert!(
                forgejo(&mut transport, url, "now")
                    .unwrap_err()
                    .to_string()
                    .contains("coordinate")
            );
        }
    }
}
