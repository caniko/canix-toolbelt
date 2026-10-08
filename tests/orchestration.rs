#![cfg(all(unix, feature = "orchestration"))]
use canix_toolbelt::orchestration::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn manifest() -> Manifest {
    serde_json::from_value(json!({"assignedPRCount":2,"baselineAssignedPRCount":2,
        "packets":[{"number":1,"sessionID":"ses_one","ownerHost":"builder","title":"Owner 1","prs":["one"]},
                   {"number":2,"sessionID":"ses_two","ownerHost":"mobile","title":"Owner 2","prs":["two"]}]})).unwrap()
}

fn policy() -> Policy {
    Policy {
        goal_policy: "finish-v1".into(),
        max_active_per_host: BTreeMap::from([("builder".into(), 1), ("mobile".into(), 1)]),
        producers: vec![],
        priority: vec![],
        expedited: vec![],
    }
}

#[test]
fn additive_repair_keeps_original_owner_and_baseline() {
    let old = manifest();
    let mut new = old.clone();
    new.packets[0].prs.push("repair".into());
    new.packets[0].linked_repairs.push("repair".into());
    new.assignment_count += 1;
    new.validate(Some(&old)).unwrap();
    new.packets[0].session_id = "ses_replacement".into();
    assert!(new.validate(Some(&old)).is_err());
}

#[test]
fn dropped_assignment_and_duplicate_registration_fail() {
    let old = manifest();
    for mutate in 0..3 {
        let mut new = old.clone();
        match mutate {
            0 => {
                new.packets[0].prs.clear();
                new.assignment_count -= 1;
            }
            1 => new.packets[1].prs[0] = "one".into(),
            _ => new.packets[1].owner_host = "builder".into(),
        }
        assert!(new.validate(Some(&old)).is_err());
    }
}

#[test]
fn pending_admission_survives_restart_with_exact_body_and_id() {
    let manifest = manifest();
    let body = json!({"id":"msg_original","text":"original text","resume":false});
    let mut state = json!({"prs":{},"packets":{"1":{"version":"new","pending":{"body":body,"version":"old","goalPolicy":"previous","preparedAt":"before"}}}});
    assert_eq!(
        prepare(
            &mut state,
            &manifest.packets[0],
            &policy(),
            "new text",
            "now"
        )
        .unwrap(),
        body
    );
    assert!(acknowledge(&mut state, 1, "msg_wrong", "now").is_err());
    acknowledge(&mut state, 1, "msg_original", "now").unwrap();
    assert_eq!(state["packets"]["1"]["deliveredVersion"], "old");
    assert_eq!(state["packets"]["1"]["deliveredPreparedAt"], "before");
}

#[test]
fn host_capacity_and_pending_inputs_bound_admission() {
    let manifest = manifest();
    let active = BTreeMap::from([("builder".into(), [1].into()), ("mobile".into(), [].into())]);
    let mut states = json!({"1":{"version":"v"},"2":{"version":"v"}});
    assert_eq!(
        select(&manifest, &states, &active, &policy()).unwrap()[0].number,
        2
    );
    states["2"]["pending"] = json!({"body":{"id":"pending"}});
    assert!(
        select(&manifest, &states, &active, &policy())
            .unwrap()
            .is_empty()
    );
    assert!(select(&manifest, &states, &BTreeMap::new(), &policy()).is_err());
}

#[test]
fn timestamps_and_errors_do_not_wake_but_edited_feedback_does() {
    let old =
        json!({"state":"OPEN","head":"h","base":"b","comments":[{"id":"same","body":"running"}]});
    let mut new = old.clone();
    new["capturedAt"] = json!("today");
    new["lastError"] = json!("502");
    assert_eq!(event_version(&old), event_version(&new));
    new["comments"][0]["body"] = json!("completed with findings");
    assert_ne!(event_version(&old), event_version(&new));
    assert_ne!(feedback_version(&old), feedback_version(&new));
}

#[test]
fn linked_repair_and_edited_historical_feedback_prevent_completion() {
    let manifest = manifest();
    let mut state = json!({"packets":{"1":{"terminal":true,"auditedFeedback":{}},"2":{"terminal":true,"auditedFeedback":{}}},
        "prs":{"one":{"url":"one","state":"CLOSED"},"two":{"url":"two","state":"MERGED"}}});
    for (number, url) in [("1", "one"), ("2", "two")] {
        state["packets"][number]["auditedFeedback"][url] =
            json!(feedback_version(&state["prs"][url]));
    }
    let active = BTreeMap::from([("builder".into(), [].into()), ("mobile".into(), [].into())]);
    assert!(finished(&manifest, &state, &active, true));
    assert!(!finished(&manifest, &state, &active, false));
    state["prs"]["one"]["comments"] = json!([{"body":"new historical finding"}]);
    assert!(!finished(&manifest, &state, &active, true));
}

#[test]
fn writer_lease_and_atomic_private_state_preserve_anchors() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let anchor = dir.path().join("review-loop.lock");
    let lease = Lease::acquire(&anchor).unwrap();
    assert!(Lease::acquire(&anchor).is_err());
    drop(lease);
    assert!(anchor.exists());
    let _lease = Lease::acquire(&anchor).unwrap();
    let state = dir.path().join("state.json");
    atomic_json(&state, &json!({"pending":{"id":"original"}})).unwrap();
    assert_eq!(
        std::fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn checkpoint_progress_deduplicates_and_stale_merge_claims_are_not_adopted() {
    let manifest = manifest();
    let mut state = json!({"prs":{"one":{"state":"OPEN","head":"h","base":"b"}},"packets":{}});
    refresh(
        &manifest,
        &mut state,
        &policy(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    state["packets"]["1"]["deliveredVersion"] = state["packets"]["1"]["version"].clone();
    let mut reports = BTreeMap::from([(
        1,
        json!({"schemaVersion":1,"packet":1,"status":"ready_for_work","eventVersion":state["packets"]["1"]["version"],"nextAction":"publish repair"}),
    )]);
    refresh(
        &manifest,
        &mut state,
        &policy(),
        &reports,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(needs_wake(&state["packets"]["1"]));
    state["packets"]["1"]["deliveredVersion"] = state["packets"]["1"]["version"].clone();
    refresh(
        &manifest,
        &mut state,
        &policy(),
        &reports,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(!needs_wake(&state["packets"]["1"]));
    reports.get_mut(&1).unwrap()["status"] = json!("waiting");
    reports.get_mut(&1).unwrap()["eventVersion"] = state["packets"]["1"]["version"].clone();
    refresh(
        &manifest,
        &mut state,
        &policy(),
        &reports,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(!needs_wake(&state["packets"]["1"]));
    let pr = &state["prs"]["one"];
    assert_ne!(
        progress(
            pr,
            &json!({"head":"old","base":"b","stage":"ready_for_merge","evidence":"receipt"})
        )["stage"],
        "ready_for_merge"
    );
    assert_ne!(
        progress(
            pr,
            &json!({"head":"h","base":"b","stage":"ready_for_merge"})
        )["stage"],
        "ready_for_merge"
    );
    assert_eq!(
        progress(
            pr,
            &json!({"head":"h","base":"b","stage":"ready_for_merge","evidence":"receipt"})
        )["stage"],
        "ready_for_merge"
    );
}

#[test]
fn old_audit_cannot_absorb_edited_feedback_or_a_new_linked_repair() {
    let mut manifest = manifest();
    let mut state = json!({"prs":{"one":{"url":"one","state":"CLOSED","head":"h","comments":[]}},"packets":{"1":{"deliveredVersion":"v","pending":{"body":{"id":"original","text":"exact"}},"unrecognized":"keep"}}});
    let reports = BTreeMap::from([(
        1,
        json!({"schemaVersion":1,"packet":1,"status":"complete","eventVersion":"v","prs":[{"url":"one","head":"h","auditComplete":true,"disposition":"withdrawn","evidence":"closure"}]}),
    )]);
    let refresh_state = |manifest: &Manifest, state: &mut Value| {
        refresh(
            manifest,
            state,
            &policy(),
            &reports,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap()
    };
    refresh_state(&manifest, &mut state);
    assert_eq!(state["packets"]["1"]["terminal"], true);
    assert_eq!(state["packets"]["1"]["pending"]["body"]["id"], "original");
    assert_eq!(state["packets"]["1"]["unrecognized"], "keep");
    state["prs"]["one"]["comments"] = json!([{"id":"new","body":"historical defect"}]);
    refresh_state(&manifest, &mut state);
    assert_eq!(state["packets"]["1"]["terminal"], false);
    manifest.packets[0].prs.push("repair".into());
    manifest.packets[0].linked_repairs.push("repair".into());
    manifest.assignment_count += 1;
    refresh_state(&manifest, &mut state);
    assert_eq!(state["packets"]["1"]["terminal"], false);
    assert_eq!(state["prs"]["repair"]["url"], "repair");
}

#[test]
fn fairness_serves_first_current_goal_then_oldest_repeat() {
    let mut manifest = manifest();
    manifest.packets[1].owner_host = "builder".into();
    let active = BTreeMap::from([("builder".into(), [].into()), ("mobile".into(), [].into())]);
    let mut states = json!({"1":{"version":"new","deliveredVersion":"old","goalAckVersion":"finish-v1","deliveredPreparedAt":"2026-10-08T07:25:00+00:00"},"2":{"version":"new","deliveredVersion":"old","goalAckVersion":"old-goal","deliveredPreparedAt":"2026-10-08T07:15:00+00:00"}});
    assert_eq!(
        select(&manifest, &states, &active, &policy()).unwrap()[0].number,
        2
    );
    states["2"]["goalAckVersion"] = json!("finish-v1");
    assert_eq!(
        select(&manifest, &states, &active, &policy()).unwrap()[0].number,
        2
    );
}

#[test]
fn waiting_followups_and_idle_recovery_are_clock_injected_and_bounded() {
    let mut ps = json!({"version":"v","deliveredVersion":"v","deliveredPreparedAt":"2026-10-08T00:00:00+00:00","observationVersion":"source","workerStatus":{"eventVersion":"v","status":"waiting","retryAfterSeconds":900}});
    assert!(!schedule_recheck(&mut ps, &policy(), 1791418000));
    assert!(schedule_recheck(&mut ps, &policy(), 1791419000));
    let before = ps["recheckVersion"].clone();
    assert!(!schedule_recheck(&mut ps, &policy(), 1791419001));
    assert_eq!(ps["recheckVersion"], before);
    let mut idle = json!({"id":"idle1","outcome":"succeeded","time":{"created":1791417610000i64}});
    assert!(!record_idle_recovery(&mut ps, &idle, 1791419000));
    ps["workerStatus"] = Value::Null;
    for id in ["idle1", "idle2", "idle3"] {
        idle["id"] = json!(id);
        assert!(record_idle_recovery(&mut ps, &idle, 1791419000));
        assert!(!record_idle_recovery(&mut ps, &idle, 1791419001));
    }
    idle["id"] = json!("idle4");
    assert!(!record_idle_recovery(&mut ps, &idle, 1791419000));
    assert!(ps["continuationBlocker"].is_object());
}

#[test]
fn terminal_target_motion_does_not_invalidate_historical_feedback_identity() {
    let old = json!({"state":"MERGED","head":"h","base":"b","checks":{"state":"PENDING"}});
    let new = json!({"state":"MERGED","head":"h","base":"next","checks":{"state":"SUCCESS"}});
    assert_eq!(event_version(&old), event_version(&new));
    assert_eq!(feedback_version(&old), feedback_version(&new));
    let old = json!({"state":"OPEN","head":"h","base":"b"});
    let new = json!({"state":"OPEN","head":"h","base":"next"});
    assert_ne!(event_version(&old), event_version(&new));
}

#[test]
fn missing_host_observation_and_malformed_state_cannot_establish_completion() {
    let manifest = manifest();
    let active = BTreeMap::from([("builder".into(), [].into())]);
    assert!(!finished(&manifest, &json!({}), &active, true));
    let mut invalid = json!({"packets":[],"prs":{}});
    assert!(
        prepare(
            &mut invalid,
            &manifest.packets[0],
            &policy(),
            "continue",
            "now"
        )
        .is_err()
    );
}

#[test]
fn legacy_sorted_ascii_fingerprints_preserve_unicode_and_float_encoding() {
    use sha2::{Digest, Sha256};
    let fixture = json!({"z":[1e-6, 1e20, -0.0, 0.0001],"a":"café 😀\u{7f}"});
    let canonical = r#"{"a":"caf\u00e9 \ud83d\ude00\u007f","z":[1e-06,1e+20,-0.0,0.0001]}"#;
    assert_eq!(
        digest(&fixture),
        format!("{:x}", Sha256::digest(canonical.as_bytes()))
    );
}

#[test]
fn malformed_checkpoint_refresh_leaves_the_last_valid_journal_intact() {
    let mut state = json!({"prs":{},"packets":{}});
    let original = state.clone();
    let reports = BTreeMap::from([(2, json!({"schemaVersion":1,"packet":999}))]);
    assert!(
        refresh(
            &manifest(),
            &mut state,
            &policy(),
            &reports,
            &BTreeMap::new(),
            &BTreeMap::new()
        )
        .is_err()
    );
    assert_eq!(state, original);
}
