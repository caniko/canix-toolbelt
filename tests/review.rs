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
        provider: "greptile".into(),
        reviewed_head: Some("a".repeat(40)),
        requested_base: Some("b".repeat(40)),
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
