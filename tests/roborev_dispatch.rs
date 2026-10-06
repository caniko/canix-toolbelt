#![cfg(feature = "review")]

use canix_toolbelt::review::{
    Candidate, CheckRequirement, Error, Intent, Policy, RoborevDispatch, RoborevRunner,
    dispatch_roborev_once,
};
use serde_json::{Value, json};

fn policy() -> Policy {
    Policy {
        provider: "roborev".into(),
        transport: "github-receipt-v1".into(),
        reviewer_id: 123,
        required_checks: vec![CheckRequirement {
            name: "review-policy".into(),
            app_id: Some(456),
        }],
        ..Policy::default()
    }
}

fn dispatch() -> RoborevDispatch {
    RoborevDispatch {
        intent: Intent {
            schema_version: 1,
            id: "request-one".into(),
            policy_digest: policy().digest().unwrap(),
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
        checkout: "/fixture/unique-request".into(),
        agent: "opencode".into(),
    }
}

#[derive(Default)]
struct Runner {
    enqueue_count: usize,
    jobs: Vec<Value>,
    unknown: bool,
    hide_jobs: bool,
}

fn job(dispatch: &RoborevDispatch) -> Value {
    json!({"id":7,"repo_path":dispatch.checkout,"git_ref":format!("{}..{}",dispatch.intent.candidate.base,dispatch.intent.candidate.head),
        "agent":dispatch.agent,"status":"done","job_type":"range","agentic":false,"prompt_prebuilt":false,"min_severity":"low"})
}

impl RoborevRunner for Runner {
    fn verify_checkout(&mut self, _: &RoborevDispatch) -> Result<(), Error> {
        Ok(())
    }
    fn jobs(&mut self, _: &RoborevDispatch) -> Result<Vec<Value>, Error> {
        Ok(if self.hide_jobs {
            vec![]
        } else {
            self.jobs.clone()
        })
    }
    fn enqueue(&mut self, dispatch: &RoborevDispatch) -> Result<u64, Error> {
        self.enqueue_count += 1;
        self.jobs.push(job(dispatch));
        if self.unknown {
            Err(Error("fixture lost enqueue response".into()))
        } else {
            Ok(7)
        }
    }
    fn saved_review(&mut self, id: u64) -> Result<Value, Error> {
        Ok(
            json!({"id":9,"job_id":id,"agent":"opencode","job":self.jobs[0],
            "structured_output":{"schema_version":2,"summary":"fixture canonical range","verdict":"pass","findings":[]}}),
        )
    }
}

#[test]
fn unknown_enqueue_recovers_one_exact_job_without_replay() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let mut runner = Runner {
        unknown: true,
        ..Runner::default()
    };
    let selected = dispatch();
    assert!(dispatch_roborev_once(&selected, &policy(), &state, &mut runner).is_err());
    let recovered = dispatch_roborev_once(&selected, &policy(), &state, &mut runner)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.job_id, Some(7));
    assert_eq!(recovered.review_id, Some(9));
    assert_eq!(runner.enqueue_count, 1);
    let again = dispatch_roborev_once(&selected, &policy(), &state, &mut runner)
        .unwrap()
        .unwrap();
    assert_eq!(again.job_id, recovered.job_id);
    assert_eq!(runner.enqueue_count, 1);
}

#[test]
fn ambiguous_or_unobservable_job_blocks_without_a_second_dispatch() {
    for ambiguity in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        let mut runner = Runner {
            unknown: true,
            ..Runner::default()
        };
        let selected = dispatch();
        assert!(dispatch_roborev_once(&selected, &policy(), &state, &mut runner).is_err());
        if ambiguity {
            runner.jobs.push(job(&selected));
            runner.jobs[1]["id"] = json!(8);
        } else {
            runner.hide_jobs = true;
        }
        assert!(dispatch_roborev_once(&selected, &policy(), &state, &mut runner).is_err());
        assert_eq!(runner.enqueue_count, 1);
    }
}

#[test]
fn reused_request_and_preexisting_checkout_history_cannot_be_adopted() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let mut runner = Runner::default();
    let selected = dispatch();
    dispatch_roborev_once(&selected, &policy(), &state, &mut runner).unwrap();
    let mut different = selected.clone();
    different.checkout = "/fixture/other-request".into();
    assert!(dispatch_roborev_once(&different, &policy(), &state, &mut runner).is_err());
    assert_eq!(runner.enqueue_count, 1);
    let other_directory = tempfile::tempdir().unwrap();
    assert!(
        dispatch_roborev_once(
            &selected,
            &policy(),
            &other_directory.path().join("state"),
            &mut runner
        )
        .is_err()
    );
    assert_eq!(runner.enqueue_count, 1);
}

#[test]
fn changed_agent_range_or_terminal_result_cannot_reuse_an_old_pass() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let mut runner = Runner::default();
    let selected = dispatch();
    assert_eq!(
        dispatch_roborev_once(&selected, &policy(), &state, &mut runner)
            .unwrap()
            .unwrap()
            .status,
        "done"
    );
    runner.jobs[0]["status"] = json!("failed");
    let failed = dispatch_roborev_once(&selected, &policy(), &state, &mut runner)
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, "failed");
    assert!(failed.review_id.is_none() && failed.document.is_none());
    runner.jobs[0]["git_ref"] = json!(selected.intent.candidate.head);
    assert!(dispatch_roborev_once(&selected, &policy(), &state, &mut runner).is_err());
    assert_eq!(runner.enqueue_count, 1);
}

#[test]
fn pending_jobs_resume_and_changed_selection_cannot_retrigger() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let mut runner = Runner {
        unknown: true,
        ..Runner::default()
    };
    let selected = dispatch();
    assert!(dispatch_roborev_once(&selected, &policy(), &state, &mut runner).is_err());
    runner.jobs[0]["status"] = json!("queued");
    assert!(
        dispatch_roborev_once(&selected, &policy(), &state, &mut runner)
            .unwrap()
            .is_none()
    );
    runner.jobs[0]["status"] = json!("running");
    assert!(
        dispatch_roborev_once(&selected, &policy(), &state, &mut runner)
            .unwrap()
            .is_none()
    );
    for mutation in ["agent", "base", "head"] {
        let mut changed = selected.clone();
        match mutation {
            "agent" => changed.agent = "codex".into(),
            "base" => changed.intent.candidate.base = "c".repeat(40),
            _ => changed.intent.candidate.head = "d".repeat(40),
        }
        assert!(dispatch_roborev_once(&changed, &policy(), &state, &mut runner).is_err());
    }
    assert_eq!(runner.enqueue_count, 1);
}

#[cfg(unix)]
#[test]
fn symlink_and_corrupt_ledgers_fail_before_daemon_effects() {
    use std::{fs, os::unix::fs::symlink};
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let mut runner = Runner::default();
    let selected = dispatch();
    dispatch_roborev_once(&selected, &policy(), &state, &mut runner).unwrap();
    let ledger = fs::read_dir(&state)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "json"))
        .unwrap();
    let protected = directory.path().join("protected");
    fs::write(&protected, "fixture must remain untouched").unwrap();
    fs::remove_file(&ledger).unwrap();
    symlink(&protected, &ledger).unwrap();
    assert!(dispatch_roborev_once(&selected, &policy(), &state, &mut runner).is_err());
    assert_eq!(
        fs::read_to_string(&protected).unwrap(),
        "fixture must remain untouched"
    );
    fs::remove_file(&ledger).unwrap();
    fs::write(&ledger, "{truncated").unwrap();
    assert!(dispatch_roborev_once(&selected, &policy(), &state, &mut runner).is_err());
    assert_eq!(runner.enqueue_count, 1);
}

#[test]
fn missing_ledger_cannot_turn_unknown_into_a_fresh_enqueue() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let mut runner = Runner {
        unknown: true,
        ..Runner::default()
    };
    let selected = dispatch();
    assert!(dispatch_roborev_once(&selected, &policy(), &state, &mut runner).is_err());
    for entry in std::fs::read_dir(&state).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "json") {
            std::fs::remove_file(path).unwrap();
        }
    }
    runner.hide_jobs = true;
    assert!(dispatch_roborev_once(&selected, &policy(), &state, &mut runner).is_err());
    assert_eq!(runner.enqueue_count, 1);
}
