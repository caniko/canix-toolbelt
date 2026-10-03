use super::{Candidate, Error, Policy, Review, Verdict, model::hex_digest};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

/// Explicit adapter capabilities; unsupported behavior must not silently fall back.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Provider identity.
    pub provider: String,
    /// Selected transport.
    pub transport: String,
    /// Draft-review support.
    pub drafts: bool,
    /// Whether the remote service honors request idempotency keys.
    pub idempotent_submission: bool,
    /// Whether reviewed source revisions can be proven.
    pub revision_attestation: bool,
}

/// Immutable write-ahead intent, also published as a forge request marker.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Intent {
    /// Marker schema version.
    pub schema_version: u32,
    /// Unique local dispatch identity, not a server-side idempotency claim.
    pub id: String,
    /// Frozen source/target comparison.
    pub candidate: Candidate,
    /// Frozen consumer policy identity.
    pub policy_digest: String,
    /// Reviews already present before dispatch, excluded from adoption.
    pub baseline_review_ids: Vec<String>,
}

impl Intent {
    /// Whether the current candidate and policy still match this dispatch.
    pub fn matches(&self, candidate: &Candidate, policy: &Policy) -> bool {
        self.schema_version == 1
            && self.candidate.head == candidate.head
            && self.candidate.base == candidate.base
            && self.candidate.url == candidate.url
            && self.candidate.source_repository == candidate.source_repository
            && self.candidate.source_branch == candidate.source_branch
            && self.candidate.target_branch == candidate.target_branch
            && policy.digest().ok().as_ref() == Some(&self.policy_digest)
    }
}

/// Complete adapter observation. Parent review identity is authoritative.
#[derive(Clone, Debug)]
pub struct Evidence {
    /// Complete set of previously submitted review identities.
    pub known_review_ids: Vec<String>,
    /// An authorized forge marker allows independent workers to adopt a run.
    pub recovered_intent: Option<Intent>,
    /// Proof that a possibly ambiguous dispatch reached the forge.
    pub request_receipt: Option<String>,
    /// The completed review associated with the selected request comparison.
    pub review: Option<Review>,
}

/// Forge-specific repository facts and CI evidence.
pub trait Forge {
    /// Read a fresh candidate from the authoritative forge.
    fn candidate(&mut self, url: &str) -> Result<Candidate, Error>;
    /// Return all CI/protection blockers for this exact comparison.
    fn checks(&mut self, candidate: &Candidate, policy: &Policy) -> Result<Vec<String>, Error>;
    /// Earliest permitted retry after a rate-limit response.
    fn next_poll_at(&self) -> Option<u64> {
        None
    }
}

/// Review-provider boundary, independent of Canix and its workspace.
pub trait Provider {
    /// Advertise selected transport and evidence guarantees.
    fn capabilities(&self) -> Capabilities;
    /// Collect a complete observation or fail explicitly.
    fn inspect(
        &mut self,
        candidate: &Candidate,
        policy: &Policy,
        intent: Option<&Intent>,
    ) -> Result<Evidence, Error>;
    /// Submit once; errors are ambiguous unless accompanied by a definitive rejection.
    fn submit(&mut self, intent: &Intent) -> Result<String, Error>;
    /// A definitive rate-limit rejection authorizes retry after this timestamp.
    fn next_poll_at(&self) -> Option<u64> {
        None
    }
}

/// Durable result returned to agents. Raw review bodies remain untrusted evidence.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// Wire contract version.
    pub schema_version: u32,
    /// Next-action category.
    pub verdict: Verdict,
    /// Freshly observed comparison.
    pub candidate: Candidate,
    /// Local request identity, if known.
    pub request_id: Option<String>,
    /// Parent provider/forge review and remaining findings.
    pub review: Option<Review>,
    /// Complete, human-readable blocker list.
    pub blockers: Vec<String>,
    /// Locally generated guidance, never a provider-supplied command.
    pub next_action: String,
    /// Earliest permitted poll timestamp, retained across invocations.
    pub next_poll_at: Option<u64>,
    /// Observation time; receipts are evidence, not reusable merge authorization.
    pub observed_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Submission {
    Unknown,
    Submitted,
    Deferred,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Run {
    intent: Intent,
    submission: Submission,
    request_receipt: Option<String>,
    outcome: Option<Outcome>,
}

/// Current Unix time; wall-clock rollback cannot authorize a previously deferred poll.
pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn outcome(
    candidate: &Candidate,
    intent: Option<&Intent>,
    verdict: Verdict,
    blockers: Vec<String>,
    review: Option<Review>,
) -> Outcome {
    Outcome {
        schema_version: 1, verdict, candidate: candidate.clone(), request_id: intent.map(|i| i.id.clone()),
        review, blockers, next_action: match verdict {
            Verdict::Ready => "Review and required CI are current. Use the guarded merge command when merge is authorized.",
            Verdict::Findings => "Triage every finding against source; fix and push authorized changes, or record an evidence-backed disposition, then rerun ensure.",
            Verdict::Pending => "Resume this same ensure request after nextPollAt; do not submit a duplicate review.",
            Verdict::Blocked => "Resolve the reported evidence or setup blocker, then resume ensure.",
            Verdict::Stale => "The comparison changed; rerun ensure for the new candidate.",
        }.into(), next_poll_at: None, observed_at: now_seconds(),
    }
}

fn save(path: &Path, run: &Run) -> Result<(), Error> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = path.with_extension(format!("{}.{nonce}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    let result = (|| -> Result<(), Error> {
        file.write_all(&serde_json::to_vec_pretty(run)?)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        if let Some(parent) = path.parent() {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Perform one bounded observation/dispatch under a per-PR kernel lock.
///
/// The lock protects only this tick, never a sleep or a Nix evaluation. Persisted
/// ambiguous submissions reconcile from forge markers and are never replayed.
pub fn ensure_once<F: Forge, P: Provider>(
    forge: &mut F,
    provider: &mut P,
    policy: &Policy,
    url: &str,
    state_dir: &Path,
    allow_submit: bool,
) -> Result<Outcome, Error> {
    policy.validate()?;
    let candidate = forge.candidate(url)?;
    candidate.validate()?;
    let key = hex_digest(candidate.url.as_bytes());
    fs::create_dir_all(state_dir)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(state_dir.join(format!("{key}.lock")))?;
    lock.try_lock_exclusive().map_err(|_| {
        Error("another review tick owns this PR; resume ensure after it completes".into())
    })?;
    let path = state_dir.join(format!("{key}.json"));
    let capabilities = provider.capabilities();
    if capabilities.provider != policy.provider
        || capabilities.transport != policy.transport
        || !capabilities.revision_attestation
        || (candidate.draft && !capabilities.drafts)
        || !candidate.open
    {
        return Ok(outcome(
            &candidate,
            None,
            Verdict::Blocked,
            vec!["provider capability or open-request contract is not satisfied".into()],
            None,
        ));
    }
    let mut run: Option<Run> = match fs::read(&path) {
        Ok(bytes) => Some(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if run
        .as_ref()
        .is_some_and(|r| !r.intent.matches(&candidate, policy))
    {
        run = None;
    }
    if let Some(previous) = run.as_ref().and_then(|r| r.outcome.as_ref()) {
        if previous.next_poll_at.is_some_and(|at| now_seconds() < at) {
            return Ok(previous.clone());
        }
    }
    let evidence = match provider.inspect(&candidate, policy, run.as_ref().map(|r| &r.intent)) {
        Ok(evidence) => evidence,
        Err(e) => {
            let mut result = outcome(
                &candidate,
                run.as_ref().map(|r| &r.intent),
                Verdict::Blocked,
                vec![e.to_string()],
                None,
            );
            result.next_poll_at = provider.next_poll_at();
            if let Some(run) = &mut run {
                run.outcome = Some(result.clone());
                save(&path, run)?;
            }
            return Ok(result);
        }
    };
    if run.is_none() {
        if let Some(intent) = evidence
            .recovered_intent
            .filter(|i| i.matches(&candidate, policy))
        {
            run = Some(Run {
                intent,
                submission: Submission::Submitted,
                request_receipt: evidence.request_receipt.clone(),
                outcome: None,
            });
        }
    }
    if run.is_none() && !allow_submit {
        return Ok(outcome(
            &candidate,
            None,
            Verdict::Blocked,
            vec!["no authorized revision-bound review request; run review ensure".into()],
            None,
        ));
    }
    let policy_digest = policy.digest()?;
    let mut run = run.unwrap_or_else(|| {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let id = hex_digest(format!("{key}:{nonce}:{}", std::process::id()).as_bytes());
        Run {
            intent: Intent {
                schema_version: 1,
                id,
                candidate: candidate.clone(),
                policy_digest,
                baseline_review_ids: evidence.known_review_ids.clone(),
            },
            submission: Submission::Deferred,
            request_receipt: None,
            outcome: None,
        }
    });
    let missing_marker =
        matches!(run.submission, Submission::Submitted) && evidence.request_receipt.is_none();
    if let Some(receipt) = evidence.request_receipt {
        run.request_receipt = Some(receipt);
        run.submission = Submission::Submitted;
    }
    let mut result;
    if let Some(review) = evidence.review {
        let verdict = policy.evaluate(&candidate, &review);
        result = outcome(&candidate, Some(&run.intent), verdict, vec![], Some(review));
        if verdict == Verdict::Ready {
            match forge.checks(&candidate, policy) {
                Ok(blockers) if blockers.is_empty() => {}
                Ok(blockers) => {
                    result.verdict = Verdict::Blocked;
                    result.blockers = blockers;
                }
                Err(e) => {
                    result.verdict = Verdict::Blocked;
                    result.blockers.push(e.to_string());
                    result.next_poll_at = forge.next_poll_at();
                }
            }
        }
    } else if missing_marker {
        result = outcome(&candidate, Some(&run.intent), Verdict::Blocked,
            vec!["submitted request marker is missing; restore the original authorized marker from the durable intent and receipt without retriggering the provider".into()], None);
    } else if matches!(run.submission, Submission::Unknown) {
        result = outcome(
            &candidate,
            Some(&run.intent),
            Verdict::Blocked,
            vec![
                "submission outcome unknown; reconcile the existing marker before another dispatch"
                    .into(),
            ],
            None,
        );
    } else if matches!(run.submission, Submission::Deferred) && allow_submit {
        let before_dispatch = forge.candidate(&candidate.url)?;
        if !run.intent.matches(&before_dispatch, policy) || !before_dispatch.open {
            return Ok(outcome(
                &before_dispatch,
                Some(&run.intent),
                Verdict::Stale,
                vec!["comparison changed before dispatch".into()],
                None,
            ));
        }
        // Save UNKNOWN before the remote effect: a crash can never turn intent
        // into permission to replay an unacknowledged submission.
        run.submission = Submission::Unknown;
        save(&path, &run)?;
        match provider.submit(&run.intent) {
            Ok(receipt) => {
                run.request_receipt = Some(receipt);
                run.submission = Submission::Submitted;
                result = outcome(
                    &candidate,
                    Some(&run.intent),
                    Verdict::Pending,
                    vec![],
                    None,
                );
            }
            Err(e) => {
                result = outcome(
                    &candidate,
                    Some(&run.intent),
                    Verdict::Blocked,
                    vec![format!("submission outcome unknown: {e}")],
                    None,
                );
                if let Some(at) = provider.next_poll_at() {
                    run.submission = Submission::Deferred;
                    result.blockers = vec![e.to_string()];
                    result.next_poll_at = Some(at);
                }
            }
        }
    } else {
        result = outcome(
            &candidate,
            Some(&run.intent),
            Verdict::Pending,
            vec![],
            None,
        );
    }
    // A change during provider collection must not authorize this candidate.
    let fresh = forge.candidate(&candidate.url)?;
    if !run.intent.matches(&fresh, policy) || !fresh.open {
        result = outcome(
            &fresh,
            Some(&run.intent),
            Verdict::Stale,
            vec!["head, base, branch or request state changed during collection".into()],
            result.review,
        );
    }
    result.next_action = outcome(
        &result.candidate,
        Some(&run.intent),
        result.verdict,
        vec![],
        None,
    )
    .next_action;
    run.outcome = Some(result.clone());
    save(&path, &run)?;
    Ok(result)
}
