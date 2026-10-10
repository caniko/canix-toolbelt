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
    json!({"id":"PR_one","url":"https://github.com/owner/repo/pull/1","state":"OPEN","headRefOid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","baseRefOid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","baseRef":{"target":{"oid":"cccccccccccccccccccccccccccccccccccccccc"}},"baseRefName":"main","updatedAt":"2026-10-10T00:00:00Z","isDraft":false,"commits":{"nodes":[{"commit":{"id":"commit_head","oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","statusCheckRollup":null}}]},"comments":{"totalCount":1,"nodes":[{"id":"historical","body":"edited feedback","author":{"__typename":"Bot","login":"chatgpt-codex-connector"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}},"reviews":{"totalCount":0,"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},"reviewThreads":{"totalCount":0,"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}})
}

#[test]
fn github_rejects_another_pull_identity_before_collecting_its_history() {
    for url in [
        Value::Null,
        json!(""),
        json!("https://github.com/owner/repo/pull/2"),
        json!("https://github.com/other/repo/pull/1"),
    ] {
        let mut p = pull();
        p["url"] = url;
        let sentinel = json!({"unexpected":"wrong pull history"});
        let mut transport = Graph(vec![
            json!({"repository":{"pullRequest":p}}),
            sentinel.clone(),
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
        assert_eq!(
            transport.0,
            vec![sentinel],
            "history for the wrong pull was collected"
        );
    }
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
    assert_eq!(got["base"], "cccccccccccccccccccccccccccccccccccccccc");
    assert_eq!(
        got["reportedBase"],
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
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
                json!({"head":{"sha":"abcdef"},"base":{"sha":"123456","ref":"main"},"state":"open","merged":false,"updated_at":"2026-10-09T00:00:00Z"}),
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
    json!({"head":{"sha":"abcdef"},"base":{"sha":"123456","ref":"main"},"state":"open","merged":false,"updated_at":"2026-10-09T00:00:00Z"})
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
fn forgejo_rejects_inline_identity_replay_across_distinct_reviews() {
    let mut transport = ForgeScript(vec![
        ("repos/owner/repo/pulls/1".into(), forge_pull()),
        (
            "repos/owner/repo/pulls/1/reviews?limit=100&page=1".into(),
            json!([{"id":1,"comments_count":1},{"id":2,"comments_count":1}]),
        ),
        (
            "repos/owner/repo/issues/1/comments?limit=100&page=1".into(),
            json!([]),
        ),
        (
            "repos/owner/repo/pulls/1/reviews/1/comments?limit=100&page=1".into(),
            json!([{"id":10,"body":"first"}]),
        ),
        (
            "repos/owner/repo/pulls/1/reviews/2/comments?limit=100&page=1".into(),
            json!([{"id":10,"body":"replayed"}]),
        ),
    ]);
    let error = forgejo(
        &mut transport,
        "https://codefloe.com/owner/repo/pulls/1",
        "now",
    )
    .unwrap_err();
    assert!(error.to_string().contains("inline comment identity"));
    assert!(transport.0.is_empty());
}

#[test]
fn forgejo_requires_a_valid_update_identity_before_collecting_history() {
    for value in [
        Value::Null,
        json!(1),
        json!({}),
        json!(""),
        json!("not a timestamp"),
    ] {
        let mut pull = forge_pull();
        pull["updated_at"] = value;
        let mut transport = ForgeScript(vec![("repos/owner/repo/pulls/1".into(), pull)]);
        let error = forgejo(
            &mut transport,
            "https://codefloe.com/owner/repo/pulls/1",
            "now",
        )
        .unwrap_err();
        assert!(error.to_string().contains("update identity"));
        assert!(transport.0.is_empty());
    }
    let mut pull = forge_pull();
    pull.as_object_mut().unwrap().remove("updated_at");
    let mut transport = ForgeScript(vec![("repos/owner/repo/pulls/1".into(), pull)]);
    assert!(
        forgejo(
            &mut transport,
            "https://codefloe.com/owner/repo/pulls/1",
            "now"
        )
        .unwrap_err()
        .to_string()
        .contains("update identity")
    );
}

#[test]
fn github_inline_identities_must_be_unique_across_distinct_threads() {
    let mut p = pull();
    p["reviewThreads"] = json!({"totalCount":2,"nodes":[{"id":"thread-one","comments":{"totalCount":1,"nodes":[{"id":"same-comment","body":"first finding"}],"pageInfo":{"hasNextPage":false}}},{"id":"thread-two","comments":{"totalCount":1,"nodes":[{"id":"same-comment","body":"replayed finding"}],"pageInfo":{"hasNextPage":false}}}],"pageInfo":{"hasNextPage":false}});
    let mut transport = Graph(vec![
        json!({"repository":{"viewerPermission":"WRITE","pullRequest":p}}),
    ]);
    let error = github(
        &mut transport,
        "https://github.com/owner/repo/pull/1",
        &json!({}),
        "now",
        0,
    )
    .unwrap_err();
    assert!(error.to_string().contains("inline comment identity"));
    assert!(transport.0.is_empty());
}

#[test]
fn forgejo_requires_base_revision_and_branch_identity_before_collecting_history() {
    for field in ["sha", "ref"] {
        for value in [Value::Null, json!(1), json!({}), json!("")] {
            let mut p = forge_pull();
            p["base"][field] = value;
            let mut transport = ForgeScript(vec![("repos/owner/repo/pulls/1".into(), p)]);
            let error = forgejo(
                &mut transport,
                "https://codefloe.com/owner/repo/pulls/1",
                "now",
            )
            .unwrap_err();
            assert!(error.to_string().contains("base identity"));
            assert!(transport.0.is_empty());
        }
    }
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

#[test]
fn github_rejects_invalid_revision_and_update_identities_before_history() {
    for pointer in [
        "/headRefOid",
        "/baseRefOid",
        "/baseRef/target/oid",
        "/updatedAt",
    ] {
        for bad in [
            Value::Null,
            json!(1),
            json!({}),
            json!(""),
            json!("invalid"),
        ] {
            let mut p = pull();
            *p.pointer_mut(pointer).unwrap() = bad;
            // Even matching empty commit/head values cannot establish identity.
            if pointer == "/headRefOid" {
                p["commits"]["nodes"][0]["commit"]["oid"] = p["headRefOid"].clone();
            }
            let sentinel = json!({"unexpected":"history fetched before identity validation"});
            let mut transport = Graph(vec![
                json!({"repository":{"pullRequest":p}}),
                sentinel.clone(),
            ]);
            let error = github(
                &mut transport,
                "https://github.com/owner/repo/pull/1",
                &json!({}),
                "now",
                0,
            )
            .unwrap_err();
            assert_eq!(
                error.kind(),
                io::ErrorKind::InvalidData,
                "{pointer}: {error}"
            );
            assert_eq!(
                transport.0,
                vec![sentinel],
                "{pointer}: fetched history for invalid identity"
            );
        }
    }
}

#[test]
fn github_malformed_check_rollups_return_errors_without_panicking() {
    for malformed in [
        json!(false),
        json!(42),
        json!("success"),
        json!([]),
        json!([{}]),
    ] {
        let mut p = pull();
        p["commits"]["nodes"][0]["commit"]["statusCheckRollup"] = malformed;
        let mut transport = Graph(vec![json!({"repository":{"pullRequest":p}})]);
        let error = github(
            &mut transport,
            "https://github.com/owner/repo/pull/1",
            &json!({}),
            "now",
            0,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

#[test]
fn forgejo_accepts_exactly_2000_history_rows_with_an_empty_sentinel_page() {
    for count in [1999usize, 2000, 2001] {
        let mut script = vec![
            ("repos/owner/repo/pulls/1".into(), forge_pull()),
            (
                "repos/owner/repo/pulls/1/reviews?limit=100&page=1".into(),
                json!([{"id":1,"comments_count":count.min(2000)}]),
            ),
            (
                "repos/owner/repo/issues/1/comments?limit=100&page=1".into(),
                json!([]),
            ),
        ];
        let last_page = if count < 2000 { 20 } else { 21 };
        for page in 1..=last_page {
            let start = (page - 1) * 100;
            let rows: Vec<_> = (start..(start + 100).min(count))
                .map(|id| json!({"id":id + 1,"body":format!("comment {id}")}))
                .collect();
            script.push((
                format!("repos/owner/repo/pulls/1/reviews/1/comments?limit=100&page={page}"),
                json!(rows),
            ));
        }
        if count <= 2000 {
            script.extend([
                (
                    "repos/owner/repo/commits/abcdef/status?limit=100&page=1".into(),
                    forge_status(),
                ),
                ("repos/owner/repo/pulls/1".into(), forge_pull()),
                (
                    "repos/owner/repo/commits/abcdef/status?limit=100&page=1".into(),
                    forge_status(),
                ),
            ]);
        }
        let mut transport = ForgeScript(script);
        let result = forgejo(
            &mut transport,
            "https://codefloe.com/owner/repo/pulls/1",
            "now",
        );
        if count <= 2000 {
            let snapshot = result.unwrap_or_else(|error| panic!("{count}: {error}"));
            assert_eq!(snapshot["inlineComments"].as_array().unwrap().len(), count);
        } else {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
        assert!(
            transport.0.is_empty(),
            "{count}: boundary page was not inspected"
        );
    }
}
