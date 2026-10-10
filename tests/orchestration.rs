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
fn retained_manifest_and_packet_extensions_cannot_be_silently_lost() {
    let mut old = manifest();
    old.extra
        .insert("launchEvidence".into(), json!({"source":"original"}));
    old.packets[0]
        .extra
        .insert("ownerEvidence".into(), json!({"identity":"retained"}));
    let mut state = json!({"prs":{},"packets":{}});
    refresh(
        &old,
        &mut state,
        &policy(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    for scope in ["manifest", "packet"] {
        let mut new = old.clone();
        if scope == "manifest" {
            new.extra.clear();
        } else {
            new.packets[0].extra.clear();
        }
        let before = state.clone();
        assert!(
            refresh(
                &new,
                &mut state,
                &policy(),
                &BTreeMap::new(),
                &BTreeMap::new(),
                &BTreeMap::new()
            )
            .is_err(),
            "retained {scope} evidence was silently discarded"
        );
        assert_eq!(state, before);
    }
}

#[test]
fn retained_pr_record_urls_must_equal_their_assignment_keys() {
    for url in [Value::Null, json!("other"), json!("")] {
        let mut state = json!({"prs":{"one":{"url":url,"state":"CLOSED"}},"packets":{}});
        let before = state.clone();
        assert!(
            refresh(
                &manifest(),
                &mut state,
                &policy(),
                &BTreeMap::new(),
                &BTreeMap::new(),
                &BTreeMap::new()
            )
            .is_err(),
            "foreign PR identity entered the retained journal"
        );
        assert_eq!(state, before);
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
fn ready_for_work_cannot_close_a_terminal_packet_before_its_continuation_is_delivered() {
    for native_state in ["MERGED", "CLOSED"] {
        let mut manifest = manifest();
        manifest.packets.truncate(1);
        manifest.assignment_count = 1;
        manifest.baseline_count = Some(1);
        let (mut state, version) = delivered_state(
            json!({"url":"one","state":native_state,"head":"h","comments":[]}),
            &manifest,
        );
        let reports = BTreeMap::from([(
            1,
            json!({"schemaVersion":1,"packet":1,"status":"ready_for_work","eventVersion":version,"nextAction":"registered follow-up","prs":[{"url":"one","head":"h","auditComplete":true,"disposition":"absorbed","evidence":"current audit"}]}),
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
        assert_eq!(state["packets"]["1"]["auditBacklog"], 0);
        assert!(state["packets"]["1"]["auditedFeedback"]["one"].is_string());
        assert_eq!(state["packets"]["1"]["terminal"], false);
        assert!(needs_wake(&state["packets"]["1"]));
        let active = BTreeMap::from([("builder".into(), [].into())]);
        assert!(!finished(&manifest, &state, &active, true));
        // Completion independently rejects an imported stale terminal flag.
        let mut stale = state.clone();
        stale["packets"]["1"]["terminal"] = json!(true);
        assert!(!finished(&manifest, &stale, &active, true));
        let body = prepare(
            &mut state,
            &manifest.packets[0],
            &policy(),
            "deliver requested follow-up",
            "2026-10-10T00:00:00Z",
        )
        .unwrap();
        acknowledge(
            &mut state,
            1,
            body["id"].as_str().unwrap(),
            "2026-10-10T00:00:01Z",
        )
        .unwrap();
        refresh(
            &manifest,
            &mut state,
            &policy(),
            &reports,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(
            !needs_wake(&state["packets"]["1"]),
            "old report must not regenerate the same continuation"
        );
    }
}

#[test]
fn idle_recovery_uses_a_delivered_boundary_with_millisecond_precision() {
    let mut ps = json!({"version":"v","deliveredVersion":"v","deliveredPreparedAt":"2026-10-08T00:00:00Z","deliveredAt":"2026-10-08T01:00:00.900Z","observationVersion":"source"});
    let idle = json!({"id":"old-idle","time":{"created":1791421200899i64}});
    assert!(!record_idle_recovery(&mut ps, &idle, 1791421800));
    assert!(ps["idleRecovery"].is_null());
    let idle = json!({"id":"new-idle","time":{"created":1791421200901i64}});
    assert!(record_idle_recovery(&mut ps, &idle, 1791421800));
    assert!(!record_idle_recovery(&mut ps, &idle, 1791421801));

    let mut delayed = json!({"version":"v","deliveredVersion":"v","deliveredPreparedAt":"2026-10-08T00:00:00Z","deliveredSubmissionAttemptAt":"2026-10-08T01:00:00.900Z","deliveredAt":"2026-10-08T01:05:00Z","observationVersion":"source"});
    assert!(
        record_idle_recovery(&mut delayed, &idle, 1791421800),
        "an idle after actual submission remains usable despite delayed receipt reconciliation"
    );
    let mut legacy =
        json!({"version":"v","deliveredVersion":"v","deliveredPreparedAt":"2026-10-08T00:00:00Z"});
    assert!(
        !record_idle_recovery(&mut legacy, &idle, 1791421800),
        "preparation alone cannot authorize idle recovery"
    );
}

#[test]
fn old_audit_cannot_absorb_edited_feedback_or_a_new_linked_repair() {
    let mut manifest = manifest();
    let (mut state, version) = delivered_state(
        json!({"url":"one","state":"CLOSED","head":"h","comments":[]}),
        &manifest,
    );
    state["packets"]["1"]["pending"] = json!({"body":{"id":"original","text":"exact"}});
    state["packets"]["1"]["unrecognized"] = json!("keep");
    let reports = BTreeMap::from([(
        1,
        json!({"schemaVersion":1,"packet":1,"status":"complete","eventVersion":version,"prs":[{"url":"one","head":"h","auditComplete":true,"disposition":"withdrawn","evidence":"closure"}]}),
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
    let mut ps = json!({"version":"v","deliveredVersion":"v","deliveredPreparedAt":"2026-10-08T00:00:00+00:00","deliveredAt":"2026-10-08T00:00:00+00:00","observationVersion":"source","workerStatus":{"eventVersion":"v","status":"waiting","retryAfterSeconds":900}});
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

#[test]
fn manifest_classification_is_immutable_and_new_work_is_explicitly_linked() {
    let old = manifest();
    let mut swapped = old.clone();
    swapped.packets[0].linked_repairs.push("one".into());
    swapped.packets[0].prs.push("replacement".into());
    swapped.assignment_count += 1;
    swapped.linked_count = Some(1);
    assert!(swapped.validate(Some(&old)).is_err());
    let mut previous = old.clone();
    previous.packets[0]
        .prs
        .extend(["repair".into(), "release".into()]);
    previous.packets[0].linked_repairs.push("repair".into());
    previous.packets[0].linked_releases.push("release".into());
    previous.assignment_count += 2;
    previous.linked_count = Some(1);
    previous.release_count = Some(1);
    previous.validate(Some(&old)).unwrap();
    let mut swapped = previous.clone();
    swapped.packets[0].linked_repairs = vec!["release".into()];
    swapped.packets[0].linked_releases = vec!["repair".into()];
    assert!(swapped.validate(Some(&previous)).is_err());
    let mut overlap = previous.clone();
    overlap.packets[0].linked_releases.push("repair".into());
    assert!(overlap.validate(None).is_err());
    let mut added_baseline = previous.clone();
    added_baseline.packets[0].prs.push("unclassified".into());
    added_baseline.assignment_count += 1;
    added_baseline.baseline_count = None;
    let mut old_without_count = previous;
    old_without_count.baseline_count = None;
    assert!(added_baseline.validate(Some(&old_without_count)).is_err());
}

#[test]
fn linked_release_coverage_keeps_the_188_baseline_and_two_repairs_separate() {
    let mut manifest = manifest();
    manifest.packets[0].prs = (0..188).map(|id| format!("baseline-{id}")).collect();
    manifest.packets[1].prs = vec!["repair-1".into(), "repair-2".into(), "release".into()];
    manifest.packets[1].linked_repairs = vec!["repair-1".into(), "repair-2".into()];
    manifest.packets[1].linked_releases = vec!["release".into()];
    manifest.assignment_count = 191;
    manifest.baseline_count = Some(188);
    manifest.linked_count = Some(2);
    manifest.release_count = Some(1);
    let mut state = json!({"prs":{},"packets":{}});
    for url in &manifest.packets[0].prs {
        state["prs"][url] = json!({"state":"MERGED"});
    }
    state["prs"]["repair-1"] = json!({"state":"CLOSED"});
    state["prs"]["repair-2"] = json!({"state":"MERGED"});
    state["prs"]["release"] = json!({"state":"OPEN"});
    let coverage = serde_json::to_value(manifest.coverage(&state).unwrap()).unwrap();
    assert_eq!(coverage["baselineAssignedPRCount"], 188);
    assert_eq!(coverage["linkedRepairPRCount"], 2);
    assert_eq!(coverage["linkedReleasePRCount"], 1);
    assert_eq!(coverage["baselineStateCounts"]["MERGED"], 188);
    assert_eq!(coverage["linkedRepairStateCounts"]["MERGED"], 1);
    assert_eq!(coverage["linkedRepairStateCounts"]["CLOSED"], 1);
    assert_eq!(coverage["linkedReleaseStateCounts"]["OPEN"], 1);
}

#[test]
fn registered_release_is_scheduled_and_adopts_only_current_release_progress() {
    let old = manifest();
    let mut manifest = old.clone();
    manifest.packets[0].prs.push("release".into());
    manifest.packets[0].linked_releases.push("release".into());
    manifest.assignment_count += 1;
    manifest.release_count = Some(1);
    let mut state = json!({"prs":{},"packets":{}});
    refresh(
        &old,
        &mut state,
        &policy(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    state["prs"]["release"] = json!({"url":"release","state":"OPEN","head":"rh","base":"rb"});
    refresh(
        &manifest,
        &mut state,
        &policy(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(needs_wake(&state["packets"]["1"]));
    let body = prepare(
        &mut state,
        &manifest.packets[0],
        &policy(),
        "qualify release",
        "now",
    )
    .unwrap();
    acknowledge(&mut state, 1, body["id"].as_str().unwrap(), "now").unwrap();
    let reports = BTreeMap::from([(
        1,
        json!({"schemaVersion":1,"packet":1,"status":"waiting","eventVersion":body["metadata"]["eventVersion"],"prs":[],"linkedReleasePRs":[{"url":"release","head":"rh","base":"rb","stage":"ready_for_merge","evidence":"current release qualification"}]}),
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
    assert_eq!(
        state["prs"]["release"]["progress"]["stage"],
        "ready_for_merge"
    );
    assert_eq!(state["coverage"]["linkedReleaseStateCounts"]["OPEN"], 1);
    assert_eq!(state["packets"]["1"]["terminal"], false);
}

#[test]
fn refresh_retains_scope_across_restart_and_rejects_dropped_owners_or_assignments() {
    let original_manifest = manifest();
    let mut state = json!({"prs":{},"packets":{}});
    refresh(
        &original_manifest,
        &mut state,
        &policy(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    let retained: Manifest = serde_json::from_value(state["retainedManifest"].clone()).unwrap();
    assert_eq!(retained.assignment_count, 2);
    for mutation in 0..3 {
        let mut changed = original_manifest.clone();
        match mutation {
            0 => {
                changed.packets.pop();
                changed.assignment_count -= 1;
                changed.baseline_count = Some(1);
            }
            1 => {
                changed.packets[0].prs = vec!["replacement".into()];
            }
            _ => changed.packets[0].session_id = "ses_replacement".into(),
        }
        let mut restarted: Value =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert!(
            refresh(
                &changed,
                &mut restarted,
                &policy(),
                &BTreeMap::new(),
                &BTreeMap::new(),
                &BTreeMap::new()
            )
            .is_err()
        );
        assert_eq!(restarted, state);
    }
    let mut legacy = json!({"prs":{"dropped":{"state":"OPEN"}},"packets":{}});
    let before = legacy.clone();
    assert!(
        refresh(
            &original_manifest,
            &mut legacy,
            &policy(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new()
        )
        .is_err()
    );
    assert_eq!(legacy, before);
}

fn delivered_state(pr: Value, manifest: &Manifest) -> (Value, Value) {
    let mut state = json!({"prs":{"one":pr},"packets":{}});
    refresh(
        manifest,
        &mut state,
        &policy(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    let body = prepare(
        &mut state,
        &manifest.packets[0],
        &policy(),
        "inspect this exact event",
        "2026-10-09T00:00:00Z",
    )
    .unwrap();
    acknowledge(
        &mut state,
        1,
        body["id"].as_str().unwrap(),
        "2026-10-09T00:00:01Z",
    )
    .unwrap();
    (state, body["metadata"]["eventVersion"].clone())
}

#[test]
fn late_terminal_report_cannot_audit_feedback_it_never_observed() {
    let manifest = manifest();
    let (mut state, delivered) = delivered_state(
        json!({"url":"one","state":"CLOSED","head":"h","comments":[]}),
        &manifest,
    );
    state["prs"]["one"]["comments"] = json!([{"id":"new","body":"new defect"}]);
    let reports = BTreeMap::from([(
        1,
        json!({"schemaVersion":1,"packet":1,"status":"complete","eventVersion":delivered,"prs":[{"url":"one","head":"h","auditComplete":true,"disposition":"absorbed","evidence":"old audit"}]}),
    )]);
    for _ in 0..2 {
        refresh(
            &manifest,
            &mut state,
            &policy(),
            &reports,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_ne!(state["packets"]["1"]["terminal"], true);
        assert!(state["packets"]["1"]["auditedFeedback"]["one"].is_null());
    }
}

#[test]
fn old_progress_never_certifies_new_feedback_checks_or_dependency_evidence() {
    for stage in ["ready_for_merge", "ready_for_close"] {
        for mutation in 0..3 {
            let mut manifest = manifest();
            manifest.packets[0].dependencies = vec![2];
            let (mut state, delivered) = delivered_state(
                json!({"url":"one","state":"OPEN","head":"h","base":"b","comments":[],"checks":{"state":"SUCCESS"}}),
                &manifest,
            );
            let reports = BTreeMap::from([(
                1,
                json!({"schemaVersion":1,"packet":1,"status":"waiting","eventVersion":delivered,"prs":[{"url":"one","head":"h","base":"b","stage":stage,"evidence":"old qualification"}]}),
            )]);
            let mut dependencies = BTreeMap::new();
            match mutation {
                0 => state["prs"]["one"]["comments"] = json!([{"body":"edited feedback"}]),
                1 => state["prs"]["one"]["checks"]["state"] = json!("FAILURE"),
                _ => {
                    dependencies.insert(2, "new producer evidence".into());
                }
            }
            for _ in 0..2 {
                refresh(
                    &manifest,
                    &mut state,
                    &policy(),
                    &reports,
                    &BTreeMap::new(),
                    &dependencies,
                )
                .unwrap();
                assert_ne!(state["prs"]["one"]["progress"]["stage"], stage);
            }
        }
    }
}

#[test]
fn fresh_source_bound_audit_replaces_stale_coverage_but_stale_inputs_do_not() {
    let manifest = manifest();
    let mut state =
        json!({"prs":{"one":{"url":"one","state":"MERGED","head":"h","comments":[]}},"packets":{}});
    let initial = feedback_version(&state["prs"]["one"]);
    let mut audits = BTreeMap::from([(
        1,
        json!({"packet":1,"prs":[{"url":"one","feedbackVersion":initial,"evidence":"original"}]}),
    )]);
    refresh(
        &manifest,
        &mut state,
        &policy(),
        &BTreeMap::new(),
        &audits,
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(state["packets"]["1"]["terminal"], true);
    state["prs"]["one"]["comments"] = json!([{"body":"edited historical finding"}]);
    refresh(
        &manifest,
        &mut state,
        &policy(),
        &BTreeMap::new(),
        &audits,
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(state["packets"]["1"]["terminal"], false);
    let current = feedback_version(&state["prs"]["one"]);
    audits.get_mut(&1).unwrap()["prs"][0]["feedbackVersion"] = json!(current);
    refresh(
        &manifest,
        &mut state,
        &policy(),
        &BTreeMap::new(),
        &audits,
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(state["packets"]["1"]["terminal"], true);
    assert_eq!(state["packets"]["1"]["auditedFeedback"]["one"], current);
    audits.get_mut(&1).unwrap()["prs"][0]["feedbackVersion"] = json!(initial);
    refresh(
        &manifest,
        &mut state,
        &policy(),
        &BTreeMap::new(),
        &audits,
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(state["packets"]["1"]["auditedFeedback"]["one"], current);
}

#[test]
fn repeated_event_versions_have_distinct_fresh_ids_and_exact_pending_retries() {
    let manifest = manifest();
    let mut state =
        json!({"prs":{},"packets":{"1":{"version":"A","observationVersion":"observed-A"}}});
    let first = prepare(
        &mut state,
        &manifest.packets[0],
        &policy(),
        "same text",
        "now",
    )
    .unwrap();
    acknowledge(&mut state, 1, first["id"].as_str().unwrap(), "now").unwrap();
    state["packets"]["1"]["version"] = json!("B");
    let second = prepare(
        &mut state,
        &manifest.packets[0],
        &policy(),
        "other event",
        "now",
    )
    .unwrap();
    acknowledge(&mut state, 1, second["id"].as_str().unwrap(), "now").unwrap();
    state["packets"]["1"]["version"] = json!("A");
    let third = prepare(
        &mut state,
        &manifest.packets[0],
        &policy(),
        "same text",
        "now",
    )
    .unwrap();
    assert_ne!(first["id"], third["id"]);
    assert_ne!(second["id"], third["id"]);
    let mut restarted: Value =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(
        prepare(
            &mut restarted,
            &manifest.packets[0],
            &policy(),
            "changed text",
            "later"
        )
        .unwrap(),
        third
    );
}

#[test]
fn repeat_fairness_compares_offsets_and_fractional_instants_chronologically() {
    let mut manifest = manifest();
    manifest.packets[1].owner_host = "builder".into();
    let active = BTreeMap::from([("builder".into(), [].into()), ("mobile".into(), [].into())]);
    for (older, newer) in [
        ("2026-10-09T08:00:00+02:00", "2026-10-09T07:00:00Z"),
        ("2026-10-09T07:00:00Z", "2026-10-09T07:00:00.100Z"),
    ] {
        let states = json!({"1":{"version":"new","deliveredVersion":"old","goalAckVersion":"finish-v1","deliveredPreparedAt":older},"2":{"version":"new","deliveredVersion":"old","goalAckVersion":"finish-v1","deliveredPreparedAt":newer}});
        assert_eq!(
            select(&manifest, &states, &active, &policy()).unwrap()[0].number,
            1
        );
    }
}

#[test]
fn journal_publication_recovers_from_a_legacy_orphan_without_clobbering_it() {
    let dir = tempfile::tempdir().unwrap();
    let value = json!({"pending":"retained"});
    let orphan = dir.path().join(format!(
        ".native-state-{}-{}",
        std::process::id(),
        digest(&value)
    ));
    std::fs::write(&orphan, b"unrelated abandoned bytes").unwrap();
    let path = dir.path().join("journal.json");
    atomic_json(&path, &value).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(path).unwrap()).unwrap(),
        value
    );
    assert_eq!(std::fs::read(orphan).unwrap(), b"unrelated abandoned bytes");
}

#[test]
fn retained_producer_dependencies_cannot_be_dropped_or_reassigned() {
    let mut original = manifest();
    original.packets[0].dependencies = vec![2];
    let mut dropped = original.clone();
    dropped.packets[0].dependencies.clear();
    assert!(dropped.validate(Some(&original)).is_err());
    let mut third = original.packets[1].clone();
    third.number = 3;
    third.session_id = "ses_third".into();
    third.prs = vec!["three".into()];
    original.packets.push(third);
    original.assignment_count += 1;
    original.baseline_count = Some(3);
    let mut replaced = original.clone();
    replaced.packets[0].dependencies = vec![3];
    assert!(replaced.validate(Some(&original)).is_err());
    let mut additive = original.clone();
    additive.packets[0].dependencies.push(3);
    additive.validate(Some(&original)).unwrap();
}

#[test]
fn bare_relative_state_paths_publish_successfully() {
    if std::env::var_os("TOOLBELT_RELATIVE_JOURNAL_CHILD").is_some() {
        let value = json!({"pending":"retained"});
        atomic_json(std::path::Path::new("state.json"), &value).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read("state.json").unwrap()).unwrap(),
            value
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "bare_relative_state_paths_publish_successfully",
            "--nocapture",
        ])
        .env("TOOLBELT_RELATIVE_JOURNAL_CHILD", "1")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dir.path().join("state.json").is_file());
}

#[test]
fn duplicate_worker_report_urls_cannot_certify_audit_or_progress() {
    for terminal in [false, true] {
        for cross_collection in [false, true] {
            let manifest = manifest();
            let (mut state, version) = delivered_state(
                json!({"url":"one","state":if terminal {"MERGED"}else{"OPEN"},"head":"h","base":"b","comments":[]}),
                &manifest,
            );
            let valid = json!({"url":"one","head":"h","base":"b","stage":"ready_for_merge","auditComplete":true,"evidence":"receipt"});
            let conflicting = json!({"url":"one","head":"h","base":"b","stage":"waiting_for_producer","reason":"unsettled defect","auditComplete":false});
            let mut report = json!({"schemaVersion":1,"packet":1,"status":"complete","eventVersion":version,"prs":[valid]});
            if cross_collection {
                report["linkedReleasePRs"] = json!([conflicting]);
            } else {
                report["prs"].as_array_mut().unwrap().push(conflicting);
            }
            let original = state.clone();
            let error = refresh(
                &manifest,
                &mut state,
                &policy(),
                &BTreeMap::from([(1, report)]),
                &BTreeMap::new(),
                &BTreeMap::new(),
            )
            .unwrap_err();
            assert!(error.contains("duplicate worker report URL"));
            assert_eq!(state, original);
        }
    }
}

#[test]
fn terminal_audits_expire_when_producer_evidence_changes() {
    let mut manifest = manifest();
    manifest.packets[0].dependencies = vec![2];
    let mut state = json!({"packets":{},"prs":{"one":{"url":"one","state":"MERGED","head":"h","comments":[]},"two":{"url":"two","state":"OPEN","head":"producer","base":"main"}}});
    let mut dependencies = BTreeMap::from([(2, "producer evidence A".to_owned())]);
    refresh(
        &manifest,
        &mut state,
        &policy(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &dependencies,
    )
    .unwrap();
    let version = state["packets"]["1"]["version"].clone();
    state["packets"]["1"]["deliveredVersion"] = version.clone();
    state["packets"]["1"]["deliveredObservationVersion"] =
        state["packets"]["1"]["observationVersion"].clone();
    let mut reports = BTreeMap::from([(
        1,
        json!({"schemaVersion":1,"packet":1,"status":"complete","eventVersion":version,"prs":[{"url":"one","head":"h","auditComplete":true,"evidence":"audited source"}]}),
    )]);
    refresh(
        &manifest,
        &mut state,
        &policy(),
        &reports,
        &BTreeMap::new(),
        &dependencies,
    )
    .unwrap();
    assert_eq!(state["packets"]["1"]["terminal"], true);
    dependencies.insert(2, "producer evidence B".to_owned());
    let historical = BTreeMap::from([(
        1,
        json!({"packet":1,"prs":[{"url":"one","feedbackVersion":feedback_version(&state["prs"]["one"]),"evidence":"old historical receipt"}]}),
    )]);
    for _ in 0..2 {
        refresh(
            &manifest,
            &mut state,
            &policy(),
            &reports,
            &historical,
            &dependencies,
        )
        .unwrap();
        assert_eq!(state["packets"]["1"]["terminal"], false);
        assert_eq!(state["prs"]["one"]["progress"]["auditComplete"], false);
        assert!(needs_wake(&state["packets"]["1"]));
    }
    let current = state["packets"]["1"]["version"].clone();
    state["packets"]["1"]["deliveredVersion"] = current.clone();
    state["packets"]["1"]["deliveredObservationVersion"] =
        state["packets"]["1"]["observationVersion"].clone();
    reports.get_mut(&1).unwrap()["eventVersion"] = current;
    refresh(
        &manifest,
        &mut state,
        &policy(),
        &reports,
        &historical,
        &dependencies,
    )
    .unwrap();
    assert_eq!(state["packets"]["1"]["terminal"], true);
}

#[test]
fn adding_a_declared_producer_without_a_handoff_is_a_new_deliverable_event() {
    let mut manifest = manifest();
    let (mut state, version) = delivered_state(
        json!({"url":"one","state":"MERGED","head":"h","comments":[]}),
        &manifest,
    );
    let reports = BTreeMap::from([(
        1,
        json!({"schemaVersion":1,"packet":1,"status":"complete","eventVersion":version,"prs":[{"url":"one","head":"h","auditComplete":true,"evidence":"current audit"}]}),
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
    assert_eq!(state["packets"]["1"]["terminal"], true);
    manifest.packets[0].dependencies.push(2);
    refresh(
        &manifest,
        &mut state,
        &policy(),
        &reports,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(state["packets"]["1"]["terminal"], false);
    assert!(
        needs_wake(&state["packets"]["1"]),
        "a newly declared producer must be observable before its handoff exists"
    );
    let body = prepare(
        &mut state,
        &manifest.packets[0],
        &policy(),
        "inspect the new producer dependency",
        "2026-10-10T00:00:00Z",
    )
    .unwrap();
    assert_ne!(body["metadata"]["eventVersion"], version);
}
