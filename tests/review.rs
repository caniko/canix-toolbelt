#![cfg(feature = "review")]

use canix_toolbelt::review::{Candidate, Finding, Policy, Review, Verdict};
use canix_toolbelt::review::{Capabilities, Error, Evidence, Forge, Intent, Provider, ensure_once};

fn candidate() -> Candidate {
    Candidate {
        url: "https://github.com/example/project/pull/12".into(),
        source_repository: "example/project".into(),
        source_branch: "feature".into(),
        target_branch: "main".into(),
        head: "a".repeat(40),
        base: "b".repeat(40),
        draft: false,
        open: true,
    }
}

fn review() -> Review {
    Review {
        id: "42".into(),
        provider_job_id: None,
        provider_review_id: None,
        provider: "greptile".into(),
        reviewed_head: Some("a".repeat(40)),
        requested_base: Some("b".repeat(40)),
        reviewed_base: None,
        policy_digest: Policy::default().digest().unwrap(),
        completed: true,
        complete_findings: true,
        findings: vec![],
        url: "https://github.com/example/project/pull/12#pullrequestreview-42".into(),
    }
}

#[test]
fn only_complete_current_review_can_be_ready() {
    let policy = Policy::default();
    assert_eq!(policy.evaluate(&candidate(), &review()), Verdict::Ready);
    let mut missing = review();
    missing.reviewed_head = None;
    assert_eq!(policy.evaluate(&candidate(), &missing), Verdict::Blocked);
    let mut partial = review();
    partial.complete_findings = false;
    assert_eq!(policy.evaluate(&candidate(), &partial), Verdict::Blocked);
    let mut pending = review();
    pending.completed = false;
    assert_eq!(policy.evaluate(&candidate(), &pending), Verdict::Pending);
}

#[test]
fn head_base_and_policy_changes_invalidate_acceptance() {
    let mut moved = candidate();
    moved.head = "c".repeat(40);
    assert_eq!(
        Policy::default().evaluate(&moved, &review()),
        Verdict::Stale
    );
    moved = candidate();
    moved.base = "c".repeat(40);
    assert_eq!(
        Policy::default().evaluate(&moved, &review()),
        Verdict::Stale
    );
    let mut changed_policy = review();
    changed_policy.policy_digest = "old-policy".into();
    assert_eq!(
        Policy::default().evaluate(&candidate(), &changed_policy),
        Verdict::Stale
    );
}

#[test]
fn addressed_status_and_unknown_severity_do_not_silence_findings() {
    let mut findings = review();
    findings.findings.push(Finding {
        id: "comment-1".into(),
        body: "Investigate this problem".into(),
        path: Some("src/lib.rs".into()),
        line: Some(12),
        severity: None,
        provider_addressed: true,
        correlated: true,
        url: "https://github.com/example/project/pull/12#discussion_r1".into(),
    });
    assert_eq!(
        Policy::default().evaluate(&candidate(), &findings),
        Verdict::Findings
    );
}

struct Fake {
    submits: usize,
    ambiguous: bool,
    delivered: bool,
    finished: bool,
}
impl Forge for Fake {
    fn candidate(&mut self, _: &str) -> Result<Candidate, Error> {
        Ok(candidate())
    }
    fn checks(&mut self, _: &Candidate, _: &Policy) -> Result<Vec<String>, Error> {
        Ok(vec![])
    }
}
impl Provider for Fake {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            provider: "greptile".into(),
            transport: "github-comment".into(),
            drafts: true,
            idempotent_submission: false,
            revision_attestation: true,
        }
    }
    fn inspect(
        &mut self,
        _: &Candidate,
        _: &Policy,
        intent: Option<&Intent>,
    ) -> Result<Evidence, Error> {
        Ok(Evidence {
            known_review_ids: vec![],
            recovered_intent: None,
            request_receipt: self.delivered.then(|| "comment-17".into()),
            review: if self.finished && intent.is_some() {
                Some(review())
            } else {
                None
            },
        })
    }
    fn submit(&mut self, _: &Intent) -> Result<String, Error> {
        self.submits += 1;
        self.delivered = true;
        if self.ambiguous {
            Err(Error("connection lost after dispatch".into()))
        } else {
            Ok("comment-17".into())
        }
    }
}

#[test]
fn an_ambiguous_submission_is_reconciled_without_replay() {
    let state = tempfile::tempdir().unwrap();
    let mut forge = Fake {
        submits: 0,
        ambiguous: false,
        delivered: false,
        finished: false,
    };
    let mut provider = Fake {
        submits: 0,
        ambiguous: true,
        delivered: false,
        finished: false,
    };
    let first = ensure_once(
        &mut forge,
        &mut provider,
        &Policy::default(),
        &candidate().url,
        state.path(),
        true,
    )
    .unwrap();
    assert_eq!(first.verdict, Verdict::Blocked);
    let second = ensure_once(
        &mut forge,
        &mut provider,
        &Policy::default(),
        &candidate().url,
        state.path(),
        true,
    )
    .unwrap();
    assert_eq!(second.verdict, Verdict::Pending);
    assert_eq!(provider.submits, 1);
    provider.finished = true;
    assert_eq!(
        ensure_once(
            &mut forge,
            &mut provider,
            &Policy::default(),
            &candidate().url,
            state.path(),
            true
        )
        .unwrap()
        .verdict,
        Verdict::Ready
    );
    assert_eq!(provider.submits, 1);
}

#[test]
fn read_only_gate_never_requests_a_review() {
    let state = tempfile::tempdir().unwrap();
    let mut forge = Fake {
        submits: 0,
        ambiguous: false,
        delivered: false,
        finished: false,
    };
    let mut provider = Fake {
        submits: 0,
        ambiguous: false,
        delivered: false,
        finished: false,
    };
    assert_eq!(
        ensure_once(
            &mut forge,
            &mut provider,
            &Policy::default(),
            &candidate().url,
            state.path(),
            false
        )
        .unwrap()
        .verdict,
        Verdict::Blocked
    );
    assert_eq!(provider.submits, 0);
}

#[test]
fn a_deleted_submitted_marker_blocks_recovery_without_replaying() {
    let state = tempfile::tempdir().unwrap();
    let mut forge = Fake {
        submits: 0,
        ambiguous: false,
        delivered: false,
        finished: false,
    };
    let mut provider = Fake {
        submits: 0,
        ambiguous: false,
        delivered: false,
        finished: false,
    };
    ensure_once(
        &mut forge,
        &mut provider,
        &Policy::default(),
        &candidate().url,
        state.path(),
        true,
    )
    .unwrap();
    provider.delivered = false;
    let result = ensure_once(
        &mut forge,
        &mut provider,
        &Policy::default(),
        &candidate().url,
        state.path(),
        true,
    )
    .unwrap();
    assert_eq!(result.verdict, Verdict::Blocked);
    assert!(result.blockers.iter().any(|b| b.contains("marker")));
    assert_eq!(provider.submits, 1);
}

struct ChangingForge(Candidate);
impl Forge for ChangingForge {
    fn candidate(&mut self, _: &str) -> Result<Candidate, Error> {
        Ok(self.0.clone())
    }
    fn checks(&mut self, _: &Candidate, _: &Policy) -> Result<Vec<String>, Error> {
        Ok(vec![])
    }
}

struct InvisibleProvider {
    attempts: Vec<Intent>,
}
impl Provider for InvisibleProvider {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            provider: "greptile".into(),
            transport: "github-comment".into(),
            drafts: true,
            idempotent_submission: false,
            revision_attestation: true,
        }
    }
    fn inspect(
        &mut self,
        _: &Candidate,
        _: &Policy,
        _: Option<&Intent>,
    ) -> Result<Evidence, Error> {
        Ok(Evidence {
            known_review_ids: vec![],
            recovered_intent: None,
            request_receipt: None,
            review: None,
        })
    }
    fn submit(&mut self, intent: &Intent) -> Result<String, Error> {
        self.attempts.push(intent.clone());
        Err(Error("connection lost after dispatch".into()))
    }
}

#[test]
fn comparison_round_trip_preserves_unknown_attempt_without_replay() {
    let state = tempfile::tempdir().unwrap();
    let mut forge = ChangingForge(candidate());
    let mut provider = InvisibleProvider { attempts: vec![] };
    let policy = Policy::default();
    let url = forge.0.url.clone();
    let first = ensure_once(&mut forge, &mut provider, &policy, &url, state.path(), true).unwrap();
    forge.0.head = "c".repeat(40);
    let second = ensure_once(&mut forge, &mut provider, &policy, &url, state.path(), true).unwrap();
    assert_ne!(first.request_id, second.request_id);
    forge.0 = candidate();
    let restored =
        ensure_once(&mut forge, &mut provider, &policy, &url, state.path(), true).unwrap();
    assert_eq!(restored.verdict, Verdict::Blocked);
    assert_eq!(provider.attempts.len(), 2);
    assert_eq!(restored.request_id, first.request_id);
}

#[test]
fn missing_journal_cannot_replay_an_unknown_attempt() {
    let state = tempfile::tempdir().unwrap();
    let mut forge = ChangingForge(candidate());
    let mut provider = InvisibleProvider { attempts: vec![] };
    let policy = Policy::default();
    let url = forge.0.url.clone();
    ensure_once(&mut forge, &mut provider, &policy, &url, state.path(), true).unwrap();
    for entry in std::fs::read_dir(state.path()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            std::fs::remove_file(path).unwrap();
        }
    }
    let result = ensure_once(&mut forge, &mut provider, &policy, &url, state.path(), true);
    assert!(result.is_err() || result.unwrap().verdict == Verdict::Blocked);
    assert_eq!(provider.attempts.len(), 1);
}

#[test]
fn fresh_read_only_visit_allows_one_later_ensure() {
    let state = tempfile::tempdir().unwrap();
    let mut forge = ChangingForge(candidate());
    let mut provider = InvisibleProvider { attempts: vec![] };
    let policy = Policy::default();
    let url = forge.0.url.clone();
    ensure_once(
        &mut forge,
        &mut provider,
        &policy,
        &url,
        state.path(),
        false,
    )
    .unwrap();
    assert!(provider.attempts.is_empty());
    ensure_once(&mut forge, &mut provider, &policy, &url, state.path(), true).unwrap();
    ensure_once(&mut forge, &mut provider, &policy, &url, state.path(), true).unwrap();
    assert_eq!(provider.attempts.len(), 1);
}

#[test]
fn intact_legacy_attempt_resumes_but_cannot_qualify_missing_comparison_history() {
    let state = tempfile::tempdir().unwrap();
    let mut forge = ChangingForge(candidate());
    let mut provider = InvisibleProvider { attempts: vec![] };
    let policy = Policy::default();
    let url = forge.0.url.clone();
    let original =
        ensure_once(&mut forge, &mut provider, &policy, &url, state.path(), true).unwrap();
    for entry in std::fs::read_dir(state.path()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            let journal: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            std::fs::write(path, serde_json::to_vec(&journal["runs"][0]).unwrap()).unwrap();
        } else if path.extension().is_some_and(|e| e == "lock") {
            std::fs::write(path, "").unwrap();
        }
    }
    let resumed =
        ensure_once(&mut forge, &mut provider, &policy, &url, state.path(), true).unwrap();
    assert_eq!(resumed.request_id, original.request_id);
    forge.0.head = "c".repeat(40);
    let changed =
        ensure_once(&mut forge, &mut provider, &policy, &url, state.path(), true).unwrap();
    assert_eq!(changed.verdict, Verdict::Blocked);
    assert!(
        changed
            .blockers
            .iter()
            .any(|b| b.contains("legacy review history"))
    );
    assert_eq!(provider.attempts.len(), 1);
}

#[test]
fn missing_legacy_history_is_not_certified_as_a_fresh_read_only_visit() {
    let state = tempfile::tempdir().unwrap();
    let mut forge = ChangingForge(candidate());
    let mut provider = InvisibleProvider { attempts: vec![] };
    let policy = Policy::default();
    let url = forge.0.url.clone();
    ensure_once(
        &mut forge,
        &mut provider,
        &policy,
        &url,
        state.path(),
        false,
    )
    .unwrap();
    for entry in std::fs::read_dir(state.path()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            std::fs::remove_file(path).unwrap();
        } else {
            std::fs::write(path, "").unwrap();
        }
    }
    assert!(ensure_once(&mut forge, &mut provider, &policy, &url, state.path(), true).is_err());
    assert!(provider.attempts.is_empty());
}

struct DefinitiveRejection {
    submits: usize,
    retry_at: Option<u64>,
}
impl Provider for DefinitiveRejection {
    fn capabilities(&self) -> Capabilities {
        InvisibleProvider { attempts: vec![] }.capabilities()
    }
    fn inspect(
        &mut self,
        _: &Candidate,
        _: &Policy,
        _: Option<&Intent>,
    ) -> Result<Evidence, Error> {
        Ok(Evidence {
            known_review_ids: vec![],
            recovered_intent: None,
            request_receipt: None,
            review: None,
        })
    }
    fn submit(&mut self, _: &Intent) -> Result<String, Error> {
        self.submits += 1;
        if self.submits == 1 {
            self.retry_at = Some(0);
            Err(Error("definitively rejected due to rate limit".into()))
        } else {
            self.retry_at = None;
            Ok("comment-17".into())
        }
    }
    fn next_poll_at(&self) -> Option<u64> {
        self.retry_at
    }
}

#[test]
fn definitive_rejection_can_retry_same_intent_but_read_only_cannot_dispatch() {
    let state = tempfile::tempdir().unwrap();
    let mut forge = ChangingForge(candidate());
    let mut provider = DefinitiveRejection {
        submits: 0,
        retry_at: None,
    };
    let policy = Policy::default();
    let url = forge.0.url.clone();
    let rejected =
        ensure_once(&mut forge, &mut provider, &policy, &url, state.path(), true).unwrap();
    ensure_once(
        &mut forge,
        &mut provider,
        &policy,
        &url,
        state.path(),
        false,
    )
    .unwrap();
    assert_eq!(provider.submits, 1);
    let submitted =
        ensure_once(&mut forge, &mut provider, &policy, &url, state.path(), true).unwrap();
    assert_eq!(submitted.verdict, Verdict::Pending);
    assert_eq!(submitted.request_id, rejected.request_id);
    assert_eq!(provider.submits, 2);
}
