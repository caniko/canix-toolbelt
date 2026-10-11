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
    json!({"id":"PR_one","url":"https://github.com/owner/repo/pull/1","state":"OPEN","mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","reviewDecision":null,"mergeCommit":null,"headRefOid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","baseRefOid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","baseRef":{"target":{"oid":"cccccccccccccccccccccccccccccccccccccccc"}},"baseRefName":"main","updatedAt":"2026-10-10T00:00:00Z","isDraft":false,"commits":{"nodes":[{"commit":{"id":"commit_head","oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","statusCheckRollup":null}}]},"comments":{"totalCount":1,"nodes":[github_comment("historical", "edited feedback")],"pageInfo":{"hasNextPage":false,"endCursor":null}},"reviews":{"totalCount":0,"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},"reviewThreads":{"totalCount":0,"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}})
}

fn github_comment(id: &str, body: &str) -> Value {
    json!({"id":id,"url":"https://github.com/owner/repo/pull/1#issuecomment-1","body":body,"author":{"__typename":"Bot","login":"chatgpt-codex-connector"},"createdAt":"2026-10-10T00:00:00Z","updatedAt":"2026-10-10T00:00:00Z","commit":null,"originalCommit":null,"pullRequestReview":null})
}

fn github_check(id: &str) -> Value {
    json!({"id":id,"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":null,"startedAt":null,"completedAt":null})
}

#[test]
fn github_bounds_inline_comments_across_all_threads() {
    let mut p = pull();
    let nodes: Vec<_> = (0..21).map(|thread| json!({"id":format!("thread_{thread}"),"comments":{"totalCount":100,"nodes":(0..100).map(|comment|json!({"id":format!("comment_{thread}_{comment}")})).collect::<Vec<_>>(),"pageInfo":{"hasNextPage":false}}})).collect();
    p["reviewThreads"] = json!({"totalCount":21,"nodes":nodes,"pageInfo":{"hasNextPage":false}});
    let mut transport = Graph(vec![
        json!({"repository":{"pullRequest":p}}),
        json!({"repository":{"pullRequest":p}}),
    ]);
    let error = github(
        &mut transport,
        "https://github.com/owner/repo/pull/1",
        &json!({}),
        "now",
        0,
    )
    .unwrap_err();
    assert!(error.to_string().contains("aggregate"), "{error}");
    assert_eq!(
        transport.0.len(),
        1,
        "oversized aggregate fetched the final observation"
    );
}

#[test]
fn github_request_budget_is_shared_across_nested_connections() {
    struct ManyThreads {
        calls: usize,
        count: usize,
    }
    impl GraphQl for ManyThreads {
        fn query(&mut self, query: &str) -> io::Result<Value> {
            self.calls += 1;
            let threads = |start: usize, more: bool| json!({"totalCount":self.count,"nodes":(start..(start+100).min(self.count)).map(|id|json!({"id":format!("thread_{id}"),"isResolved":false,"isOutdated":false,"path":"src/lib.rs","comments":{"totalCount":1,"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":"inline"}}})).collect::<Vec<_>>(),"pageInfo":{"hasNextPage":more,"endCursor":"threads"}});
            if self.calls == 1 {
                let mut p = pull();
                p["reviewThreads"] = threads(0, true);
                return Ok(json!({"repository":{"pullRequest":p}}));
            }
            if query.contains("reviewThreads(first:100,after:") {
                return Ok(
                    json!({"repository":{"pullRequest":{"reviewThreads":threads(100,false)}}}),
                );
            }
            if query.contains("node(id:") {
                Ok(
                    json!({"node":{"comments":{"totalCount":1,"nodes":[github_comment(&format!("comment_{}",self.calls), "finding")],"pageInfo":{"hasNextPage":false}}}}),
                )
            } else {
                Ok(json!({"repository":{"pullRequest":pull()}}))
            }
        }
    }
    for count in [125, 126, 200] {
        let mut transport = ManyThreads { calls: 0, count };
        let result = github(
            &mut transport,
            "https://github.com/owner/repo/pull/1",
            &json!({}),
            "now",
            0,
        );
        if count == 125 {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            assert!(error.to_string().contains("request budget"), "{error}");
        }
        assert_eq!(transport.calls, 128);
    }
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
fn github_revalidates_every_published_status_and_target_field() {
    for (field, changed) in [
        ("mergeable", json!("CONFLICTING")),
        ("mergeStateStatus", json!("DIRTY")),
        ("reviewDecision", json!("CHANGES_REQUESTED")),
        ("isDraft", json!(true)),
        ("baseRefName", json!("another-target")),
    ] {
        let p = pull();
        let mut fresh = p.clone();
        fresh[field] = changed;
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
            .is_err(),
            "stale {field} was published"
        );
    }
}

fn complete_pull() -> Value {
    let mut p = pull();
    p["mergeable"] = json!("MERGEABLE");
    p["mergeStateStatus"] = json!("CLEAN");
    p["reviewDecision"] = Value::Null;
    p["comments"]["nodes"][0]["url"] = json!("https://github.com/owner/repo/pull/1#issuecomment-1");
    p["comments"]["nodes"][0]["createdAt"] = json!("2026-10-10T00:00:00Z");
    p["comments"]["nodes"][0]["updatedAt"] = json!("2026-10-10T00:00:00Z");
    p["reviews"] = json!({"totalCount":1,"nodes":[{"id":"review","url":"https://github.com/owner/repo/pull/1#pullrequestreview-1","author":{"login":"reviewer","__typename":"User"},"body":"review body","state":"APPROVED","commit":{"oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"submittedAt":"2026-10-10T00:00:00Z"}],"pageInfo":{"hasNextPage":false}});
    let mut inline = p["comments"]["nodes"][0].clone();
    inline["id"] = json!("inline");
    inline["commit"] = p["reviews"]["nodes"][0]["commit"].clone();
    inline["originalCommit"] = inline["commit"].clone();
    inline["pullRequestReview"] = json!({"url":"https://github.com/owner/repo/pull/1#pullrequestreview-1","commit":inline["commit"]});
    p["reviewThreads"] = json!({"totalCount":1,"nodes":[{"id":"thread","isResolved":false,"isOutdated":false,"path":"src/lib.rs","comments":{"totalCount":1,"nodes":[inline],"pageInfo":{"hasNextPage":false}}}],"pageInfo":{"hasNextPage":false}});
    p["commits"]["nodes"][0]["commit"]["statusCheckRollup"] = json!({"state":"SUCCESS","contexts":{"totalCount":2,"nodes":[{"id":"check","__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":null,"startedAt":null,"completedAt":null},{"id":"context","__typename":"StatusContext","context":"legacy","state":"SUCCESS","targetUrl":null,"createdAt":"2026-10-10T00:00:00Z"}],"pageInfo":{"hasNextPage":false}}});
    p
}

#[test]
fn github_rejects_missing_and_malformed_required_scheduling_fields() {
    for field in [
        "state",
        "isDraft",
        "baseRefName",
        "mergeable",
        "mergeStateStatus",
        "reviewDecision",
    ] {
        for bad in [
            None,
            Some(Value::Null),
            Some(json!({})),
            Some(json!("invalid")),
            Some(json!("")),
        ] {
            if field == "reviewDecision" && bad == Some(Value::Null) {
                continue;
            }
            if field == "baseRefName" && bad == Some(json!("invalid")) {
                continue; // A nonempty branch named "invalid" is native-valid.
            }
            let mut p = complete_pull();
            if let Some(bad) = bad {
                p[field] = bad;
            } else {
                p.as_object_mut().unwrap().remove(field);
            }
            let mut transport = Graph(vec![
                json!({"repository":{"pullRequest":p}}),
                json!({"repository":{"pullRequest":p}}),
            ]);
            assert!(
                github(
                    &mut transport,
                    "https://github.com/owner/repo/pull/1",
                    &json!({}),
                    "now",
                    0
                )
                .is_err(),
                "accepted unknown {field}"
            );
        }
    }
}

#[test]
fn github_rejects_truncated_history_and_check_nodes() {
    for (connection, fields) in [
        (
            "comment",
            vec!["url", "body", "author", "createdAt", "updatedAt"],
        ),
        (
            "review",
            vec!["url", "body", "author", "state", "commit", "submittedAt"],
        ),
        ("thread", vec!["isResolved", "isOutdated", "path"]),
        (
            "inline",
            vec![
                "url",
                "body",
                "author",
                "createdAt",
                "updatedAt",
                "commit",
                "originalCommit",
                "pullRequestReview",
            ],
        ),
        (
            "check",
            vec![
                "__typename",
                "name",
                "status",
                "conclusion",
                "detailsUrl",
                "startedAt",
                "completedAt",
            ],
        ),
        (
            "context",
            vec!["__typename", "context", "state", "targetUrl", "createdAt"],
        ),
    ] {
        for field in fields {
            let mut p = complete_pull();
            let node = match connection {
                "comment" => &mut p["comments"]["nodes"][0],
                "review" => &mut p["reviews"]["nodes"][0],
                "thread" => &mut p["reviewThreads"]["nodes"][0],
                "inline" => &mut p["reviewThreads"]["nodes"][0]["comments"]["nodes"][0],
                _ => {
                    &mut p["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["nodes"]
                        [usize::from(connection == "context")]
                }
            };
            node.as_object_mut().unwrap().remove(field);
            let mut transport = Graph(vec![
                json!({"repository":{"pullRequest":p}}),
                json!({"repository":{"pullRequest":p}}),
            ]);
            assert!(
                github(
                    &mut transport,
                    "https://github.com/owner/repo/pull/1",
                    &json!({}),
                    "now",
                    0
                )
                .is_err(),
                "accepted truncated {connection}.{field}"
            );
        }
    }
}

#[test]
fn forgejo_revalidates_every_published_scheduling_field() {
    for field in ["mergeable", "draft"] {
        let mut transport = Forge {
            calls: vec![],
            inconsistent: false,
        };
        struct Moving<'a> {
            inner: &'a mut Forge,
            field: &'a str,
            reads: usize,
        }
        impl ForgeRest for Moving<'_> {
            fn get(&mut self, host: &str, path: &str) -> io::Result<ForgeResponse> {
                let mut value = self.inner.get(host, path)?;
                if path == "repos/owner/repo/pulls/1" {
                    self.reads += 1;
                    value.body["draft"] = json!(false);
                    value.body["mergeable"] = json!(false);
                    if self.reads > 1 {
                        value.body[self.field] = json!(true);
                    }
                }
                Ok(value)
            }
        }
        assert!(
            forgejo(
                &mut Moving {
                    inner: &mut transport,
                    field,
                    reads: 0
                },
                "https://codefloe.com/owner/repo/pulls/1",
                "now"
            )
            .is_err(),
            "published stale {field}"
        );
    }
}

#[test]
fn forgejo_rejects_history_without_declared_transport_totals() {
    let transport = Forge {
        calls: vec![],
        inconsistent: false,
    };
    struct Bare(Forge);
    impl ForgeRest for Bare {
        fn get(&mut self, host: &str, path: &str) -> io::Result<ForgeResponse> {
            let mut response = self.0.get(host, path)?;
            response.total_count = None;
            Ok(response)
        }
    }
    assert!(
        forgejo(
            &mut Bare(transport),
            "https://codefloe.com/owner/repo/pulls/1",
            "now"
        )
        .is_err(),
        "bare short arrays certified complete history without native count metadata"
    );
}

#[test]
fn forgejo_rejects_truncated_history_and_status_rows() {
    for field in [
        "body",
        "user",
        "state",
        "submitted_at",
        "updated_at",
        "commit_id",
    ] {
        let mut review = forge_review_row(1, 1);
        review.as_object_mut().unwrap().remove(field);
        let mut transport = forge_review(review, json!([forge_inline(10, 1)]));
        assert!(
            forgejo(
                &mut transport,
                "https://codefloe.com/owner/repo/pulls/1",
                "now"
            )
            .is_err(),
            "accepted truncated Forgejo review.{field}"
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
    p["reviewThreads"] = json!({"totalCount":1,"nodes":[{"id":"thread","isResolved":false,"isOutdated":true,"path":"src/lib.rs","comments":{"totalCount":1,"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":"cursor"}}}],"pageInfo":{"hasNextPage":false,"endCursor":null}});
    let next = json!({"node":{"comments":{"totalCount":1,"nodes":[github_comment("old", "outdated finding")],"pageInfo":{"hasNextPage":false,"endCursor":null}}}});
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
    fn get(&mut self, host: &str, path: &str) -> io::Result<ForgeResponse> {
        assert_eq!(host, "codefloe.com");
        self.calls.push(path.into());
        if path == "repos/owner/repo/pulls/1" {
            return Ok(native(forge_pull()));
        }
        if path.contains("/status?") {
            return Ok(native(if path.ends_with("page=1") {
                json!({"sha":"abcdef","state":"success","total_count":101,"statuses":(0..100).map(forge_check).collect::<Vec<_>>()})
            } else {
                json!({"sha":"abcdef","state":if self.inconsistent { "failure" } else { "success" },"total_count":101,"statuses":[forge_check(100)]})
            }));
        }
        Ok(ForgeResponse {
            body: json!([]),
            total_count: Some(0),
        })
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
    p["commits"]["nodes"][0]["commit"]["statusCheckRollup"] = json!({"state":"SUCCESS","contexts":{"totalCount":1,"nodes":[github_check("check")],"pageInfo":{"hasNextPage":false}}});
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
        .map(|id| github_check(&format!("check_{id}")))
        .collect();
    p["commits"]["nodes"][0]["commit"]["statusCheckRollup"] = json!({"state":"SUCCESS","contexts":{"totalCount":101,"nodes":first,"pageInfo":{"hasNextPage":true,"endCursor":"last"}}});
    let last = json!({"node":{"statusCheckRollup":{"contexts":{"totalCount":101,"nodes":[github_check("check_last")],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
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

struct ForgeScript(
    Vec<(String, Value)>,
    std::collections::BTreeMap<String, u64>,
);

fn native(body: Value) -> ForgeResponse {
    ForgeResponse {
        body,
        total_count: None,
    }
}

fn forge_script(pages: Vec<(String, Value)>) -> ForgeScript {
    // Declare the whole scripted endpoint inventory once, independently of the
    // page returned by get. Malformed-count tests override these declarations.
    let mut totals = std::collections::BTreeMap::<String, u64>::new();
    for (path, body) in &pages {
        if let Some(rows) = body.as_array() {
            *totals
                .entry(path.split('?').next().unwrap().into())
                .or_default() += rows.len() as u64;
        }
    }
    ForgeScript(pages, totals)
}
impl ForgeRest for ForgeScript {
    fn get(&mut self, host: &str, path: &str) -> io::Result<ForgeResponse> {
        assert_eq!(host, "codefloe.com");
        if self.0.is_empty() {
            return Err(io::Error::other("unexpected extra forge request"));
        }
        let (expected, value) = self.0.remove(0);
        assert_eq!(path, expected);
        Ok(ForgeResponse {
            body: value,
            total_count: self.1.get(path.split('?').next().unwrap()).copied(),
        })
    }
}

fn forge_pull() -> Value {
    json!({"number":1,"html_url":"https://codefloe.com/owner/repo/pulls/1","head":{"sha":"abcdef"},"base":{"sha":"123456","ref":"main"},"state":"open","merged":false,"draft":false,"mergeable":true,"updated_at":"2026-10-09T00:00:00Z"})
}

#[test]
fn forgejo_binds_the_returned_pull_identity_before_history_collection() {
    for (field, bad) in [
        ("number", json!(2)),
        ("number", Value::Null),
        ("html_url", json!("https://codefloe.com/other/repo/pulls/1")),
        ("html_url", Value::Null),
    ] {
        let mut p = forge_pull();
        p[field] = bad;
        let mut transport = forge_script(vec![
            ("repos/owner/repo/pulls/1".into(), p),
            (
                "repos/owner/repo/pulls/1/reviews?limit=100&page=1".into(),
                json!([]),
            ),
        ]);
        assert!(
            forgejo(
                &mut transport,
                "https://codefloe.com/owner/repo/pulls/1",
                "now"
            )
            .is_err()
        );
        assert_eq!(transport.0.len(), 1, "wrong pull collected history");
    }
}

#[test]
fn forgejo_bounds_inline_comments_across_all_reviews_before_pagination() {
    struct LargeForge {
        calls: usize,
    }
    impl ForgeRest for LargeForge {
        fn get(&mut self, _: &str, path: &str) -> io::Result<ForgeResponse> {
            self.calls += 1;
            if path == "repos/owner/repo/pulls/1" {
                return Ok(native(forge_pull()));
            }
            if path.contains("/status?") {
                return Ok(native(forge_status()));
            }
            if path == "repos/owner/repo/pulls/1/reviews?limit=100&page=1" {
                return Ok(ForgeResponse {
                    total_count: Some(21),
                    body: json!(
                        (1..=21)
                            .map(|id| json!({"id":id,"comments_count":100}))
                            .collect::<Vec<_>>()
                    ),
                });
            }
            if let Some(id) = path
                .strip_prefix("repos/owner/repo/pulls/1/reviews/")
                .and_then(|s| s.split('/').next())
                .and_then(|s| s.parse::<u64>().ok())
            {
                if path.ends_with("page=1") {
                    return Ok(ForgeResponse {
                        total_count: Some(100),
                        body: json!(
                            (1..=100)
                                .map(|c| json!({"id":id*1000+c}))
                                .collect::<Vec<_>>()
                        ),
                    });
                }
            }
            Ok(ForgeResponse {
                body: json!([]),
                total_count: Some(0),
            })
        }
    }
    let mut transport = LargeForge { calls: 0 };
    let error = forgejo(
        &mut transport,
        "https://codefloe.com/owner/repo/pulls/1",
        "now",
    )
    .unwrap_err();
    assert!(error.to_string().contains("aggregate"), "{error}");
    assert!(
        transport.calls <= 3,
        "oversized aggregate launched inline pagination"
    );
}

#[test]
fn forgejo_request_budget_is_shared_across_nested_reviews() {
    struct ManyReviews {
        calls: usize,
        count: usize,
    }
    impl ForgeRest for ManyReviews {
        fn get(&mut self, _: &str, path: &str) -> io::Result<ForgeResponse> {
            self.calls += 1;
            if path == "repos/owner/repo/pulls/1" {
                return Ok(native(forge_pull()));
            }
            for (page, start) in [(1, 0), (2, 100)] {
                if path == format!("repos/owner/repo/pulls/1/reviews?limit=100&page={page}") {
                    return Ok(ForgeResponse {
                        total_count: Some(self.count as u64),
                        body: json!(
                            (start..(start + 100).min(self.count))
                                .map(|id| forge_review_row(id + 1, 1))
                                .collect::<Vec<_>>()
                        ),
                    });
                }
            }
            if path == "repos/owner/repo/pulls/1/reviews?limit=100&page=3" {
                return Ok(ForgeResponse {
                    body: json!([]),
                    total_count: Some(self.count as u64),
                });
            }
            if path.contains("/comments?limit=") && path.contains("/reviews/") {
                let review_id = path.split('/').nth(6).unwrap().parse().unwrap();
                return Ok(ForgeResponse {
                    body: json!([forge_inline(self.calls as u64, review_id)]),
                    total_count: Some(1),
                });
            }
            if path.contains("/status?") {
                return Ok(native(forge_status()));
            }
            Ok(ForgeResponse {
                body: json!([]),
                total_count: Some(0),
            })
        }
    }
    for count in [121, 122, 123, 200] {
        let mut transport = ManyReviews { calls: 0, count };
        let result = forgejo(
            &mut transport,
            "https://codefloe.com/owner/repo/pulls/1",
            "now",
        );
        if count == 121 {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            assert!(error.to_string().contains("request budget"), "{error}");
        }
        assert_eq!(transport.calls, 128);
    }
}

#[test]
fn forgejo_rejects_final_identity_substitution_without_other_movement() {
    for (field, bad) in [
        ("number", json!(2)),
        ("html_url", json!("https://codefloe.com/other/repo/pulls/1")),
    ] {
        let mut final_pull = forge_pull();
        final_pull[field] = bad;
        let mut transport = forge_script(vec![
            ("repos/owner/repo/pulls/1".into(), forge_pull()),
            (
                "repos/owner/repo/pulls/1/reviews?limit=100&page=1".into(),
                json!([]),
            ),
            (
                "repos/owner/repo/issues/1/comments?limit=100&page=1".into(),
                json!([]),
            ),
            (
                "repos/owner/repo/commits/abcdef/status?limit=100&page=1".into(),
                forge_status(),
            ),
            ("repos/owner/repo/pulls/1".into(), final_pull),
            (
                "repos/owner/repo/commits/abcdef/status?limit=100&page=1".into(),
                forge_status(),
            ),
        ]);
        let error = forgejo(
            &mut transport,
            "https://codefloe.com/owner/repo/pulls/1",
            "now",
        )
        .unwrap_err();
        assert!(error.to_string().contains("moved"), "{error}");
        assert_eq!(
            transport.0.len(),
            1,
            "substituted pull fetched final checks"
        );
    }
}

fn forge_status() -> Value {
    json!({"sha":"abcdef","state":"success","total_count":1,"statuses":[forge_check(1)]})
}

fn forge_check(id: u64) -> Value {
    json!({"id":id,"status":"success","context":"test","description":"done","target_url":"","url":"","creator":null,"created_at":"2026-10-10T00:00:00Z","updated_at":"2026-10-10T00:00:00Z"})
}

fn forge_review_row(id: usize, count: usize) -> Value {
    json!({"id":id,"comments_count":count,"body":"review","user":null,"state":"APPROVED","submitted_at":"2026-10-10T00:00:00Z","updated_at":"2026-10-10T00:00:00Z","commit_id":"abcdef","html_url":"","pull_request_url":"","stale":false,"official":false,"dismissed":false})
}

fn forge_inline(id: u64, review: u64) -> Value {
    json!({"id":id,"body":"inline","user":null,"created_at":"2026-10-10T00:00:00Z","updated_at":"2026-10-10T00:00:00Z","html_url":"","commit_id":"abcdef","original_commit_id":"abcdef","pull_request_review_id":review,"path":"src/lib.rs","pull_request_url":""})
}

fn forge_review(review: Value, inline: Value) -> ForgeScript {
    forge_script(vec![
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
    for inline in [
        json!([]),
        json!([forge_inline(10, 1)]),
        json!([forge_inline(10, 1), forge_inline(11, 1)]),
    ] {
        let should_pass = inline.as_array().unwrap().len() == 2;
        let mut transport = forge_review(forge_review_row(1, 2), inline);
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
    let mut replay = forge_script(vec![
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
        forge_review_row(1, 2),
        json!([forge_inline(10, 1), forge_inline(10, 1)]),
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
    let mut transport = forge_script(vec![
        ("repos/owner/repo/pulls/1".into(), forge_pull()),
        (
            "repos/owner/repo/pulls/1/reviews?limit=100&page=1".into(),
            json!([forge_review_row(1, 1), forge_review_row(2, 1)]),
        ),
        (
            "repos/owner/repo/issues/1/comments?limit=100&page=1".into(),
            json!([]),
        ),
        (
            "repos/owner/repo/pulls/1/reviews/1/comments?limit=100&page=1".into(),
            json!([forge_inline(10, 1)]),
        ),
        (
            "repos/owner/repo/pulls/1/reviews/2/comments?limit=100&page=1".into(),
            json!([forge_inline(10, 2)]),
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
        let mut transport = forge_script(vec![("repos/owner/repo/pulls/1".into(), pull)]);
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
    let mut transport = forge_script(vec![("repos/owner/repo/pulls/1".into(), pull)]);
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
    p["reviewThreads"] = json!({"totalCount":2,"nodes":[{"id":"thread-one","isResolved":false,"isOutdated":false,"path":"src/lib.rs","comments":{"totalCount":1,"nodes":[github_comment("same-comment", "first finding")],"pageInfo":{"hasNextPage":false}}},{"id":"thread-two","isResolved":false,"isOutdated":false,"path":"src/lib.rs","comments":{"totalCount":1,"nodes":[github_comment("same-comment", "replayed finding")],"pageInfo":{"hasNextPage":false}}}],"pageInfo":{"hasNextPage":false}});
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
            let mut transport = forge_script(vec![("repos/owner/repo/pulls/1".into(), p)]);
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
    let mut transport = forge_review(forge_review_row(1, 1), json!([forge_inline(10, 1)]));
    let mut moved = forge_status();
    moved["state"] = json!("failure");
    moved["statuses"][0]["status"] = json!("failure");
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
            let mut transport = forge_script(vec![]);
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
                json!([forge_review_row(1, count.min(2000))]),
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
                .map(|id| forge_inline((id + 1) as u64, 1))
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
        let mut transport = forge_script(script);
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
        if count <= 2000 {
            assert!(
                transport.0.is_empty(),
                "{count}: boundary page was not inspected"
            );
        } else {
            assert!(
                !transport.0.is_empty(),
                "oversized declared total must reject before further pages"
            );
        }
    }
}

#[test]
fn github_preserves_explicit_native_nulls_and_pending_checks() {
    let mut p = complete_pull();
    p["comments"]["nodes"][0]["author"] = Value::Null;
    p["reviews"]["nodes"][0]["author"] = Value::Null;
    p["reviews"]["nodes"][0]["commit"] = Value::Null;
    p["reviews"]["nodes"][0]["submittedAt"] = Value::Null;
    let inline = &mut p["reviewThreads"]["nodes"][0]["comments"]["nodes"][0];
    for field in ["author", "commit", "originalCommit", "pullRequestReview"] {
        inline[field] = Value::Null;
    }
    let check =
        &mut p["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["nodes"][0];
    check["status"] = json!("QUEUED");
    check["conclusion"] = Value::Null;
    let mut transport = Graph(vec![
        json!({"repository":{"pullRequest":p}}),
        json!({"repository":{"pullRequest":p}}),
    ]);
    let snapshot = github(
        &mut transport,
        "https://github.com/owner/repo/pull/1",
        &json!({}),
        "now",
        0,
    )
    .unwrap();
    assert!(snapshot["issueComments"][0]["author"].is_null());
    assert!(snapshot["reviews"][0]["submittedAt"].is_null());
    assert_eq!(
        snapshot["checks"]["contexts"]["nodes"][0]["status"],
        "QUEUED"
    );
    assert!(transport.0.is_empty());
}

#[test]
fn github_permission_requires_native_shape_and_final_consistency() {
    struct NativeGraph(Vec<Value>);
    impl GraphQl for NativeGraph {
        fn query(&mut self, _: &str) -> io::Result<Value> {
            Ok(self.0.remove(0))
        }
    }
    let p = complete_pull();
    for permission in [
        None,
        Some(json!({})),
        Some(json!("invalid")),
        Some(json!(false)),
    ] {
        let mut response = json!({"repository":{"pullRequest":p}});
        if let Some(permission) = permission {
            response["repository"]["viewerPermission"] = permission;
        }
        let mut transport = NativeGraph(vec![response.clone(), response]);
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
    let mut transport = NativeGraph(vec![
        json!({"repository":{"viewerPermission":"WRITE","pullRequest":p}}),
        json!({"repository":{"viewerPermission":"READ","pullRequest":p}}),
    ]);
    assert!(
        github(
            &mut transport,
            "https://github.com/owner/repo/pull/1",
            &json!({}),
            "now",
            0
        )
        .is_err(),
        "published stale write permission"
    );
}

#[test]
fn provider_specific_pull_segments_reject_aliases_before_transport() {
    let mut transport = Graph(vec![
        json!({"repository":{"pullRequest":complete_pull()}}),
        json!({"repository":{"pullRequest":complete_pull()}}),
    ]);
    assert!(
        github(
            &mut transport,
            "https://github.com/owner/repo/pulls/1",
            &json!({}),
            "now",
            0
        )
        .is_err()
    );
    assert_eq!(transport.0.len(), 2);
    let mut transport = forge_review(forge_review_row(1, 1), json!([forge_inline(10, 1)]));
    let original = transport.0.len();
    assert!(
        forgejo(
            &mut transport,
            "https://codefloe.com/owner/repo/pull/1",
            "now"
        )
        .is_err()
    );
    assert_eq!(transport.0.len(), original);
}

#[test]
fn github_rejects_malformed_present_history_fields() {
    for pointer in [
        "/comments/nodes/0/author/login",
        "/comments/nodes/0/body",
        "/comments/nodes/0/createdAt",
        "/reviews/nodes/0/state",
        "/reviews/nodes/0/commit/oid",
        "/reviews/nodes/0/submittedAt",
        "/reviewThreads/nodes/0/isResolved",
        "/reviewThreads/nodes/0/comments/nodes/0/pullRequestReview/commit/oid",
        "/commits/nodes/0/commit/statusCheckRollup/state",
        "/commits/nodes/0/commit/statusCheckRollup/contexts/nodes/0/status",
        "/commits/nodes/0/commit/statusCheckRollup/contexts/nodes/0/conclusion",
        "/commits/nodes/0/commit/statusCheckRollup/contexts/nodes/1/state",
    ] {
        let mut p = complete_pull();
        *p.pointer_mut(pointer).unwrap() = json!({});
        let mut transport = Graph(vec![
            json!({"repository":{"pullRequest":p}}),
            json!({"repository":{"pullRequest":p}}),
        ]);
        assert!(
            github(
                &mut transport,
                "https://github.com/owner/repo/pull/1",
                &json!({}),
                "now",
                0
            )
            .is_err(),
            "accepted {pointer}"
        );
    }
}

#[test]
fn forgejo_rejects_incomplete_or_moving_declared_history_counts() {
    for (total, second_total, first_len) in [
        (2, 2, 1),
        (101, 102, 100),
        (101, 100, 100),
        (2001, 2001, 100),
    ] {
        let mut script = forge_review(forge_review_row(1, 1), json!([forge_inline(10, 1)]));
        let path = "repos/owner/repo/pulls/1/reviews";
        script.1.insert(path.into(), total);
        script.0[1].1 = json!(
            (0..first_len)
                .map(|id| forge_review_row(id + 1, 0))
                .collect::<Vec<_>>()
        );
        struct MovingCount {
            inner: ForgeScript,
            second_total: u64,
        }
        impl ForgeRest for MovingCount {
            fn get(&mut self, host: &str, path: &str) -> io::Result<ForgeResponse> {
                if path.ends_with("/reviews?limit=100&page=2") {
                    return Ok(ForgeResponse {
                        body: json!([forge_review_row(101, 0)]),
                        total_count: Some(self.second_total),
                    });
                }
                self.inner.get(host, path)
            }
        }
        let mut transport = MovingCount {
            inner: script,
            second_total,
        };
        assert_eq!(
            forgejo(
                &mut transport,
                "https://codefloe.com/owner/repo/pulls/1",
                "now"
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidData
        );
    }
}

#[test]
fn forgejo_rejects_truncated_issue_inline_and_native_status_fields() {
    for (kind, fields) in [
        (
            "issue",
            vec!["body", "user", "created_at", "updated_at", "html_url"],
        ),
        (
            "inline",
            vec![
                "body",
                "user",
                "created_at",
                "updated_at",
                "html_url",
                "commit_id",
                "original_commit_id",
                "pull_request_review_id",
                "path",
                "pull_request_url",
            ],
        ),
        (
            "status",
            vec![
                "status",
                "context",
                "description",
                "creator",
                "created_at",
                "updated_at",
                "target_url",
                "url",
            ],
        ),
    ] {
        for field in fields {
            let mut transport = forge_review(forge_review_row(1, 1), json!([forge_inline(10, 1)]));
            if kind == "issue" {
                transport.0[2].1 = json!([forge_inline(20, 1)]);
                transport
                    .1
                    .insert("repos/owner/repo/issues/1/comments".into(), 1);
            }
            let row = match kind {
                "issue" => &mut transport.0[2].1[0],
                "inline" => &mut transport.0[3].1[0],
                _ => &mut transport.0[4].1["statuses"][0],
            };
            row.as_object_mut().unwrap().remove(field);
            assert!(
                forgejo(
                    &mut transport,
                    "https://codefloe.com/owner/repo/pulls/1",
                    "now"
                )
                .is_err(),
                "accepted {kind}.{field}"
            );
        }
    }
    let mut transport = forge_review(forge_review_row(1, 1), json!([forge_inline(10, 2)]));
    assert!(
        forgejo(
            &mut transport,
            "https://codefloe.com/owner/repo/pulls/1",
            "now"
        )
        .unwrap_err()
        .to_string()
        .contains("parent review")
    );
}
