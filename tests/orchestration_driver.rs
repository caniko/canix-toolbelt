#![cfg(all(unix, feature = "orchestration"))]

use canix_toolbelt::orchestration::{Manifest, Policy, driver::*};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io};

struct Fixture {
    hosts: BTreeMap<String, Value>,
    inbox: BTreeMap<String, Vec<Value>>,
    messages: BTreeMap<String, Value>,
    saved: Value,
    submissions: Vec<Value>,
    ambiguous: bool,
    fail_save: bool,
    changed_model: bool,
    recent: Vec<Value>,
    mirror_inbox: bool,
    start_during_inbox: bool,
    context_pages: BTreeMap<String, Value>,
    requests: Vec<String>,
}

impl Adapter for Fixture {
    fn api(
        &mut self,
        host: &str,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> io::Result<Reply> {
        self.requests.push(path.to_owned());
        if path == "/api/session/active" {
            return Ok(Reply {
                status: 200,
                data: self.hosts[host].clone(),
            });
        }
        let id = path.split('/').nth(3).unwrap().to_owned();
        if host == "source" && path.ends_with("/inbox") {
            return Ok(Reply {
                status: 200,
                data: if self.mirror_inbox {
                    json!([{"id":"msg_mirror"}])
                } else {
                    json!([])
                },
            });
        }
        let data = if path.ends_with("/inbox") {
            if self.start_during_inbox {
                self.hosts
                    .insert("destination".into(), json!({"ses_original":{}}));
            }
            json!(self.inbox.get(&id).cloned().unwrap_or_default())
        } else if let Some((_, message)) = path.split_once("/message/") {
            return Ok(Reply {
                status: if self.messages.contains_key(message) {
                    200
                } else {
                    404
                },
                data: self.messages.get(message).cloned().unwrap_or(Value::Null),
            });
        } else if let Some(page) = self.context_pages.get(path) {
            page.clone()
        } else if path.contains("/message?") && path.contains("type=") {
            let kind = if path.contains("type=assistant") {
                "assistant"
            } else {
                "compaction"
            };
            json!({"data":self.recent.iter().filter(|message| message["type"] == kind).take(64).collect::<Vec<_>>(),"cursor":{"next":null,"previous":null}})
        } else if path.contains("/message?") {
            json!(self.recent.iter().take(12).collect::<Vec<_>>())
        } else if method == "POST" && path.ends_with("/compact") {
            let body = body.unwrap();
            assert_eq!(
                self.saved["packets"]["1"]["pendingCompaction"]["body"],
                *body
            );
            self.submissions.push(body.clone());
            self.inbox
                .entry(id.clone())
                .or_default()
                .push(json!({"id":body["id"],"sessionID":id,"type":"compact"}));
            json!({"id":body["id"],"sessionID":id,"type":"compact"})
        } else if method == "POST" && path.ends_with("/prompt") {
            let body = body.unwrap();
            assert_eq!(
                self.saved["packets"]["1"]["pending"]["body"], *body,
                "input must be durable before network mutation"
            );
            self.submissions.push(body.clone());
            self.inbox.entry(id.clone()).or_default().push(json!({"id":body["id"],"sessionID":id,"type":"user","payload":{"text":body["text"],"metadata":body["metadata"]}}));
            if self.ambiguous {
                return Err(io::Error::other("lost POST response"));
            }
            json!({"id":body["id"],"sessionID":id})
        } else {
            json!({"id":id,"title":"PR review one","model":{"providerID":"test","id":if self.changed_model { "replacement" } else { "model" },"variant":"high"},"agent":"automatic","permissions":[]})
        };
        Ok(Reply { status: 200, data })
    }
    fn persist(&mut self, state: &Value) -> io::Result<()> {
        if self.fail_save {
            return Err(io::Error::other("disk unavailable"));
        }
        self.saved = state.clone();
        Ok(())
    }
    fn continuation(
        &mut self,
        _: &canix_toolbelt::orchestration::Packet,
        _: &Value,
    ) -> io::Result<String> {
        Ok("continue original owner".into())
    }
}

fn setup() -> (Manifest, Policy, Expectations, Value, Fixture) {
    let manifest = serde_json::from_value(json!({"assignedPRCount":1,"baselineAssignedPRCount":1,"packets":[{"number":1,"sessionID":"ses_original","ownerHost":"destination","title":"PR review one","prs":["one"]}]})).unwrap();
    let policy = Policy {
        goal_policy: "goal".into(),
        max_active_per_host: BTreeMap::from([("source".into(), 1), ("destination".into(), 1)]),
        producers: vec![],
        priority: vec![],
        expedited: vec![],
    };
    let expected = BTreeMap::from([(
        1,
        Identity {
            model: json!({"providerID":"test","id":"model","variant":"high"}),
            agent: "automatic".into(),
            permissions: json!([]),
        },
    )]);
    let state = json!({"prs":{},"packets":{"1":{"version":"new","observationVersion":"source"}}});
    let fixture = Fixture {
        hosts: BTreeMap::from([
            ("source".into(), json!({})),
            ("destination".into(), json!({})),
        ]),
        inbox: BTreeMap::new(),
        messages: BTreeMap::new(),
        saved: state.clone(),
        submissions: vec![],
        ambiguous: false,
        fail_save: false,
        changed_model: false,
        recent: vec![assistant("msg_known_context", 1000, 100)],
        mirror_inbox: false,
        start_during_inbox: false,
        context_pages: BTreeMap::new(),
        requests: vec![],
    };
    (manifest, policy, expected, state, fixture)
}

#[test]
fn lost_response_then_inbox_receipt_preserves_exact_admission_without_duplicate() {
    let (manifest, policy, expected, mut state, mut fixture) = setup();
    fixture.ambiguous = true;
    assert!(
        cycle(
            &manifest,
            &policy,
            &expected,
            &mut state,
            &mut fixture,
            "2026-10-08T00:00:00Z",
            1791417600,
            true
        )
        .unwrap()
        .errors
        .len()
            == 1
    );
    let pending = state["packets"]["1"]["pending"].clone();
    assert!(pending["body"]["model"].is_null());
    fixture.ambiguous = false;
    cycle(
        &manifest,
        &policy,
        &expected,
        &mut state,
        &mut fixture,
        "2026-10-08T00:00:01Z",
        1791417601,
        true,
    )
    .unwrap();
    assert!(state["packets"]["1"]["pending"].is_null());
    assert_eq!(
        state["packets"]["1"]["deliveredVersion"],
        pending["version"]
    );
    assert_eq!(fixture.submissions.len(), 1);
}

#[test]
fn active_mirror_and_queued_input_block_admission() {
    let (manifest, policy, expected, mut state, mut fixture) = setup();
    fixture
        .hosts
        .insert("source".into(), json!({"ses_original":{}}));
    assert!(
        cycle(
            &manifest,
            &policy,
            &expected,
            &mut state,
            &mut fixture,
            "now",
            0,
            true
        )
        .is_err()
    );
    assert!(fixture.submissions.is_empty());
    fixture.hosts.insert("source".into(), json!({}));
    fixture.mirror_inbox = true;
    assert!(
        cycle(
            &manifest,
            &policy,
            &expected,
            &mut state,
            &mut fixture,
            "now",
            0,
            true
        )
        .is_err()
    );
    fixture.mirror_inbox = false;
    fixture
        .inbox
        .insert("ses_original".into(), vec![json!({"id":"foreign_input"})]);
    let report = cycle(
        &manifest,
        &policy,
        &expected,
        &mut state,
        &mut fixture,
        "now",
        0,
        true,
    )
    .unwrap();
    assert!(report.selected.is_empty());
    assert!(fixture.submissions.is_empty());
}

#[test]
fn activity_starting_during_inbox_collection_cannot_admit_a_second_input() {
    let (manifest, policy, expected, mut state, mut fixture) = setup();
    let original = state.clone();
    fixture.start_during_inbox = true;
    assert!(
        cycle(
            &manifest,
            &policy,
            &expected,
            &mut state,
            &mut fixture,
            "now",
            0,
            true,
        )
        .is_err()
    );
    assert!(fixture.submissions.is_empty());
    assert_eq!(fixture.saved, original);
}

#[test]
fn omitted_attachment_fields_cannot_acknowledge_added_capabilities() {
    for key in ["files", "agents", "skills"] {
        for in_inbox in [false, true] {
            let (manifest, policy, expected, mut state, mut fixture) = setup();
            let body = json!({"id":"msg_original_input","text":"exact historical input","resume":true,"metadata":{"source":"old"}});
            state["packets"]["1"]["pending"] =
                json!({"body":body,"version":"old","goalPolicy":"old-goal","preparedAt":"before"});
            let mut payload = json!({"text":body["text"],"metadata":body["metadata"]});
            payload[key] = json!(["unexpected capability"]);
            let receipt =
                json!({"id":body["id"],"sessionID":"ses_original","type":"user","payload":payload});
            if in_inbox {
                fixture.inbox.insert("ses_original".into(), vec![receipt]);
            } else {
                fixture
                    .messages
                    .insert("msg_original_input".into(), receipt);
            }
            assert!(
                cycle(
                    &manifest,
                    &policy,
                    &expected,
                    &mut state,
                    &mut fixture,
                    "now",
                    0,
                    true,
                )
                .is_err(),
                "unexpected {key} in receipt (inbox={in_inbox})"
            );
            assert_eq!(state["packets"]["1"]["pending"]["body"], body);
            assert!(fixture.submissions.is_empty());
        }
    }
}

#[test]
fn explicit_empty_receipt_attachments_match_an_omitted_body() {
    let (manifest, policy, expected, mut state, mut fixture) = setup();
    let body = json!({"id":"msg_original_input","text":"exact historical input","resume":true,"metadata":{"source":"old"}});
    state["packets"]["1"]["version"] = json!("old");
    state["packets"]["1"]["pending"] =
        json!({"body":body,"version":"old","goalPolicy":"old-goal","preparedAt":"before"});
    fixture.messages.insert("msg_original_input".into(), json!({"id":body["id"],"type":"user","text":body["text"],"metadata":body["metadata"],"files":[],"agents":[],"skills":[]}));
    cycle(
        &manifest,
        &policy,
        &expected,
        &mut state,
        &mut fixture,
        "now",
        0,
        true,
    )
    .unwrap();
    assert!(state["packets"]["1"]["pending"].is_null());
    assert_eq!(state["packets"]["1"]["deliveredVersion"], "old");
    assert!(fixture.submissions.is_empty());
}

#[test]
fn persistence_failure_cannot_submit_and_shadow_cannot_write() {
    let (manifest, policy, expected, mut state, mut fixture) = setup();
    fixture.fail_save = true;
    assert!(
        cycle(
            &manifest,
            &policy,
            &expected,
            &mut state,
            &mut fixture,
            "now",
            0,
            true
        )
        .is_err()
    );
    assert!(fixture.submissions.is_empty());
    let original = state.clone();
    cycle(
        &manifest,
        &policy,
        &expected,
        &mut state,
        &mut fixture,
        "now",
        0,
        false,
    )
    .unwrap();
    assert_eq!(state, original);
    assert!(fixture.submissions.is_empty());
}

#[test]
fn transcript_receipt_without_session_id_is_adopted_but_edited_body_is_rejected() {
    let (manifest, policy, expected, mut state, mut fixture) = setup();
    let body = json!({"id":"msg_original_input","text":"exact historical input","resume":true,"metadata":{"source":"old"}});
    state["packets"]["1"]["version"] = json!("old");
    state["packets"]["1"]["pending"] =
        json!({"body":body,"version":"old","goalPolicy":"old-goal","preparedAt":"before"});
    fixture.messages.insert("msg_original_input".into(),json!({"id":"msg_original_input","type":"user","text":"edited","metadata":{"source":"old"}}));
    assert!(
        cycle(
            &manifest,
            &policy,
            &expected,
            &mut state,
            &mut fixture,
            "now",
            0,
            true
        )
        .is_err()
    );
    assert_eq!(state["packets"]["1"]["pending"]["body"], body);
    fixture.messages.get_mut("msg_original_input").unwrap()["text"] = body["text"].clone();
    cycle(
        &manifest,
        &policy,
        &expected,
        &mut state,
        &mut fixture,
        "now",
        0,
        true,
    )
    .unwrap();
    assert!(state["packets"]["1"]["pending"].is_null());
    assert_eq!(state["packets"]["1"]["deliveredVersion"], "old");
    assert!(fixture.submissions.is_empty());
}

#[test]
fn changed_model_or_context_ceiling_cannot_admit_work() {
    let (manifest, policy, expected, mut state, mut fixture) = setup();
    fixture.changed_model = true;
    assert!(
        cycle(
            &manifest,
            &policy,
            &expected,
            &mut state,
            &mut fixture,
            "now",
            0,
            true
        )
        .is_err()
    );
    fixture.changed_model = false;
    fixture.recent = vec![
        json!({"id":"msg_physical","type":"assistant","time":{"created":1000},"tokens":{"input":299999,"cache":{"read":1,"write":0}}}),
    ];
    assert!(
        cycle(
            &manifest,
            &policy,
            &expected,
            &mut state,
            &mut fixture,
            "now",
            0,
            true
        )
        .is_err()
    );
    assert!(fixture.submissions.is_empty());
}

#[test]
fn cache_write_only_contexts_compact_or_refuse_at_the_hard_ceiling() {
    for tokens in [180_000, 300_000] {
        let (manifest, policy, expected, mut state, mut fixture) = setup();
        fixture.recent = vec![
            json!({"id":"msg_cache_write_only","type":"assistant","time":{"created":1000},"tokens":{"input":0,"cache":{"read":0,"write":tokens}}}),
        ];
        let result = cycle(
            &manifest,
            &policy,
            &expected,
            &mut state,
            &mut fixture,
            "now",
            0,
            true,
        );
        if tokens == 300_000 {
            assert!(result.is_err());
            assert!(fixture.submissions.is_empty());
        } else {
            result.unwrap();
            assert_eq!(fixture.submissions.len(), 1);
            assert!(fixture.submissions[0]["text"].is_null());
            assert_eq!(
                state["packets"]["1"]["pendingCompaction"]["physicalMessageID"],
                "msg_cache_write_only"
            );
        }
    }
}

#[test]
fn context_compaction_uses_one_durable_id_before_continuation() {
    let (manifest, policy, expected, mut state, mut fixture) = setup();
    fixture.recent = vec![
        json!({"id":"msg_physical","type":"assistant","time":{"created":1000},"tokens":{"input":180000,"cache":{"read":0,"write":0}}}),
    ];
    cycle(
        &manifest,
        &policy,
        &expected,
        &mut state,
        &mut fixture,
        "now",
        0,
        true,
    )
    .unwrap();
    assert_eq!(fixture.submissions.len(), 1);
    assert!(fixture.submissions[0]["text"].is_null());
    let id = fixture.submissions[0]["id"].as_str().unwrap().to_owned();
    cycle(
        &manifest,
        &policy,
        &expected,
        &mut state,
        &mut fixture,
        "now",
        0,
        true,
    )
    .unwrap();
    assert_eq!(fixture.submissions.len(), 1);
    fixture.inbox.clear();
    let completed =
        json!({"id":id,"type":"compaction","status":"completed","time":{"created":2000}});
    fixture.messages.insert(id.clone(), completed.clone());
    fixture.recent.insert(0, completed);
    cycle(
        &manifest,
        &policy,
        &expected,
        &mut state,
        &mut fixture,
        "now",
        0,
        true,
    )
    .unwrap();
    assert!(state["packets"]["1"]["pendingCompaction"].is_null());
    assert_eq!(fixture.submissions.len(), 2);
    assert_eq!(fixture.submissions[1]["text"], "continue original owner");
}

fn assistant(id: &str, created: u64, tokens: u64) -> Value {
    json!({"id":id,"type":"assistant","time":{"created":created},"tokens":{"input":tokens,"cache":{"read":0,"write":0}}})
}

#[test]
fn hidden_physical_context_still_compacts_or_rejects_the_ceiling() {
    for tokens in [180_000, 300_000] {
        let (manifest, policy, expected, mut state, mut fixture) = setup();
        fixture.recent = (0..30).map(|index| json!({"id":format!("msg_status_{index}"),"type":"system","time":{"created":2000-index}})).collect();
        fixture
            .recent
            .push(assistant("msg_empty_assistant", 1500, 0));
        fixture
            .recent
            .push(assistant("msg_hidden_physical", 1000, tokens));
        let result = cycle(
            &manifest,
            &policy,
            &expected,
            &mut state,
            &mut fixture,
            "now",
            0,
            true,
        );
        if tokens == 300_000 {
            assert!(result.unwrap_err().to_string().contains("context ceiling"));
            assert!(fixture.submissions.is_empty());
        } else {
            result.unwrap();
            assert_eq!(fixture.submissions.len(), 1);
            assert_eq!(
                state["packets"]["1"]["pendingCompaction"]["physicalMessageID"],
                "msg_hidden_physical"
            );
        }
    }
}

#[test]
fn unknown_or_malformed_usage_cannot_admit_or_publish_state() {
    for tokens in [
        Value::Null,
        json!({}),
        json!({"input":100}),
        json!({"input":"100","cache":{"read":0,"write":0}}),
        json!({"input":100,"cache":{"read":null,"write":0}}),
        json!({"input":-1,"cache":{"read":0,"write":0}}),
        json!({"input":100,"cache":{"read":0,"write":0.5}}),
        json!({"input":u64::MAX,"cache":{"read":1,"write":0}}),
    ] {
        for dispatch in [false, true] {
            let (manifest, policy, expected, mut state, mut fixture) = setup();
            let original = state.clone();
            fixture.recent[0]["tokens"] = tokens.clone();
            assert!(
                cycle(
                    &manifest,
                    &policy,
                    &expected,
                    &mut state,
                    &mut fixture,
                    "now",
                    0,
                    dispatch
                )
                .is_err()
            );
            assert_eq!(state, original);
            assert_eq!(fixture.saved, original);
            assert!(fixture.submissions.is_empty());
        }
    }
    for recent in [vec![], vec![assistant("msg_no_usage", 1000, 0)]] {
        let (manifest, policy, expected, mut state, mut fixture) = setup();
        fixture.recent = recent;
        let original = state.clone();
        let error = cycle(
            &manifest,
            &policy,
            &expected,
            &mut state,
            &mut fixture,
            "now",
            0,
            true,
        )
        .unwrap_err();
        assert!(error.to_string().contains("physical context is unknown"));
        assert_eq!(state, original);
        assert!(fixture.submissions.is_empty());
    }
}

#[test]
fn filtered_context_pagination_retains_the_actual_physical_identity() {
    let (manifest, policy, expected, _, mut fixture) = setup();
    let first: Vec<_> = (0..64)
        .map(|index| assistant(&format!("msg_empty_{index}"), 2000 - index, 0))
        .collect();
    fixture.context_pages.insert(
        "/api/session/ses_original/message?limit=64&order=desc&type=assistant".into(),
        json!({"data":first,"cursor":{"next":"older/page?"}}),
    );
    fixture.context_pages.insert("/api/session/ses_original/message?limit=64&type=assistant&cursor=%6F%6C%64%65%72%2F%70%61%67%65%3F".into(), json!({"data":[assistant("msg_physical_page2", 1000, 180_000)],"cursor":{"next":null}}));
    let observation = observe(&manifest, &policy, &expected, &mut fixture).unwrap();
    assert_eq!(observation.context_tokens[&1], 180_000);
    assert_eq!(observation.physical_message_ids[&1], "msg_physical_page2");
    assert_eq!(
        fixture
            .requests
            .iter()
            .filter(|path| path.contains("type=assistant"))
            .count(),
        2
    );
}

#[test]
fn only_valid_completed_compaction_releases_observed_context() {
    for status in ["completed", "failed", "running"] {
        let (manifest, policy, expected, _, mut fixture) = setup();
        fixture.recent = vec![
            json!({"id":"msg_compacted","type":"compaction","status":status,"time":{"created":2000}}),
            assistant("msg_retained", 1000, 180_000),
        ];
        let observation = observe(&manifest, &policy, &expected, &mut fixture).unwrap();
        assert_eq!(
            observation.context_tokens[&1],
            if status == "completed" { 0 } else { 180_000 }
        );
        assert_eq!(observation.physical_message_ids[&1], "msg_retained");
    }
    for invalid in [
        json!({"id":"msg_compacted","type":"compaction","status":"completed"}),
        json!({"id":"msg_compacted","type":"compaction","status":"unknown","time":{"created":2000}}),
    ] {
        let (manifest, policy, expected, mut state, mut fixture) = setup();
        fixture.recent.insert(0, invalid);
        assert!(
            cycle(
                &manifest,
                &policy,
                &expected,
                &mut state,
                &mut fixture,
                "now",
                0,
                true
            )
            .is_err()
        );
        assert!(fixture.submissions.is_empty());
    }
}

#[test]
fn physical_context_search_exhaustion_is_bounded_and_never_zero() {
    let (manifest, policy, expected, mut state, mut fixture) = setup();
    for page in 0..4 {
        let path = if page == 0 {
            "/api/session/ses_original/message?limit=64&order=desc&type=assistant".to_owned()
        } else {
            format!(
                "/api/session/ses_original/message?limit=64&type=assistant&cursor=%3{}",
                page - 1
            )
        };
        let messages: Vec<_> = (0..64)
            .map(|index| {
                assistant(
                    &format!("msg_empty_{page}_{index}"),
                    2000 - page * 64 - index,
                    0,
                )
            })
            .collect();
        fixture.context_pages.insert(
            path,
            json!({"data":messages,"cursor":{"next":page.to_string()}}),
        );
    }
    let original = state.clone();
    let error = cycle(
        &manifest,
        &policy,
        &expected,
        &mut state,
        &mut fixture,
        "now",
        0,
        true,
    )
    .unwrap_err();
    assert!(error.to_string().contains("bounded history"));
    assert_eq!(
        fixture
            .requests
            .iter()
            .filter(|path| path.contains("type=assistant"))
            .count(),
        4
    );
    assert_eq!(state, original);
    assert!(fixture.submissions.is_empty());
}
