//! Local-daemon review collection for explicitly prepared PR and advisory scopes.
use super::{
    Candidate, Capabilities, Error, Evidence, Finding, Intent, Policy, Provider, Review,
    RoborevHttp,
};
use super::{
    model::hex_digest,
    roborev_dispatch::{claim, read_ledger_with_limit},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

/// Consumer-selected comparison scope. Local scopes never attest a forge PR range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoborevLocalScope {
    /// Require the full freshly observed target-tip..source-tip comparison.
    PullRequest,
    /// Consumer-prepared committed comparison; advisory for a forge PR.
    Committed,
    /// Exact captured tracked diff in a consumer-prepared snapshot; advisory.
    WorkingTree,
}

/// Frozen consumer inputs. Callers verify snapshot custody and trusted worker
/// configuration before constructing a provider; the transport does not do that.
pub struct RoborevLocalRequest {
    /// Request-exclusive immutable checkout, never the personal working tree.
    pub snapshot: PathBuf,
    /// Full base..head range, or `dirty` for captured working-tree input.
    pub git_ref: String,
    /// Explicit approved adapter name.
    pub agent: String,
    /// Exact captured tracked diff, only for a working-tree review.
    pub diff: Option<String>,
    /// Number of changed files independently counted by the consumer.
    pub expected_files: usize,
    /// Whether this is a full PR comparison or a local advisory review.
    pub scope: RoborevLocalScope,
}

// Preserve the standalone client's legacy JSON fields and SHA-256 filenames.
// The new lock is persistent, including after a missing/ambiguous reply.
#[derive(Serialize, Deserialize)]
struct Submission {
    intent: Intent,
    snapshot: PathBuf,
    git_ref: String,
    baseline: Vec<String>,
    job: Option<Value>,
    dispatching: bool,
    diff_digest: Option<String>,
}

fn read_submission(path: &std::path::Path, fresh: bool) -> Result<Option<Submission>, Error> {
    // Legacy local journals can contain the captured diff inside saved job data.
    read_ledger_with_limit(path, fresh, 16 * 1024 * 1024)
}

/// Direct-daemon provider with durable enqueue recovery. This interface supplies
/// local evidence; dedicated-App forge publication uses `RoborevReceipt` instead.
pub struct RoborevLocal {
    daemon: RoborevHttp,
    request: RoborevLocalRequest,
    state: PathBuf,
}

impl RoborevLocal {
    /// Bind already-qualified worker policy and prepared immutable inputs.
    pub fn new(
        daemon: RoborevHttp,
        request: RoborevLocalRequest,
        state: PathBuf,
    ) -> Result<Self, Error> {
        if !request.snapshot.is_absolute()
            || !state.is_absolute()
            || !matches!(request.agent.as_str(), "opencode" | "codex" | "claude-code")
            || request.git_ref.is_empty()
            || request.git_ref.chars().any(char::is_control)
            || request.expected_files == 0
            || (request.scope == RoborevLocalScope::WorkingTree) != request.diff.is_some()
            || (request.scope == RoborevLocalScope::WorkingTree && request.git_ref != "dirty")
        {
            return Err(Error("invalid frozen local roborev selection".into()));
        }
        Ok(Self {
            daemon,
            request,
            state,
        })
    }

    fn key(intent: &Intent) -> String {
        hex_digest(intent.id.as_bytes())
    }
    fn matches(&self, job: &Value) -> bool {
        job["repo_path"].as_str() == self.request.snapshot.to_str()
            && job["git_ref"].as_str() == Some(&self.request.git_ref)
            && job["agent"].as_str() == Some(&self.request.agent)
            && job["agentic"].as_bool() == Some(false)
            && job["min_severity"]
                .as_str()
                .is_some_and(|s| s.is_empty() || s == "low")
            && job.get("panel_run_uuid").is_none_or(Value::is_null)
            && job["backup_agent"].as_str().is_none_or(str::is_empty)
    }
    fn matches_diff(&self, job: &Value) -> bool {
        self.request
            .diff
            .as_ref()
            .is_none_or(|diff| job["diff_content"].as_str() == Some(diff))
    }
    fn detail(&self, listed: &Value) -> Result<Value, Error> {
        if self.request.diff.is_none() {
            return Ok(listed.clone());
        }
        let data =
            self.daemon
                .request("GET", &format!("/api/jobs?id={}", job_id(listed)?), None)?;
        let jobs = data["jobs"]
            .as_array()
            .ok_or_else(|| Error("working-tree job detail missing".into()))?;
        if jobs.len() != 1 || data["has_more"].as_bool() != Some(false) {
            return Err(Error("working-tree job detail missing or ambiguous".into()));
        }
        let job = &jobs[0];
        if !self.matches(job)
            || !self.matches_diff(job)
            || job["id"] != listed["id"]
            || job["uuid"] != listed["uuid"]
        {
            return Err(Error("working-tree job diff or identity mismatch".into()));
        }
        Ok(job.clone())
    }
    fn checkout(&self) -> Result<&str, Error> {
        self.request
            .snapshot
            .to_str()
            .ok_or_else(|| Error("snapshot path is not UTF-8".into()))
    }
}

fn job_id(job: &Value) -> Result<String, Error> {
    job["id"]
        .as_u64()
        .filter(|id| *id > 0)
        .map(|id| id.to_string())
        .ok_or_else(|| Error("job ID missing".into()))
}
fn receipt(job: &Value) -> Result<String, Error> {
    let uuid = job["uuid"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error("job UUID missing".into()))?;
    Ok(format!("roborev:job:{}:{uuid}", job_id(job)?))
}
fn verify_comparison(
    git_ref: &str,
    candidate: &Candidate,
    scope: RoborevLocalScope,
) -> Result<(), Error> {
    candidate.validate()?;
    if scope == RoborevLocalScope::WorkingTree {
        return if git_ref == "dirty" {
            Ok(())
        } else {
            Err(Error(
                "working-tree review requires the captured dirty comparison".into(),
            ))
        };
    }
    if scope == RoborevLocalScope::PullRequest
        && git_ref != format!("{}..{}", candidate.base, candidate.head)
    {
        return Err(Error(
            "PR review must compare the full observed target SHA to the source SHA".into(),
        ));
    }
    let (base, head) = git_ref
        .split_once("..")
        .ok_or_else(|| Error("review comparison must contain two full commit IDs".into()))?;
    if !matches!(base.len(), 40 | 64)
        || !base.bytes().all(|byte| byte.is_ascii_hexdigit())
        || head != candidate.head
    {
        return Err(Error(
            "review comparison must attest the actual base and intended source SHA".into(),
        ));
    }
    Ok(())
}

impl Provider for RoborevLocal {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            provider: "roborev".into(),
            transport: "unix-http-v0.71".into(),
            drafts: true,
            idempotent_submission: false,
            revision_attestation: true,
        }
    }
    fn inspect(
        &mut self,
        candidate: &Candidate,
        policy: &Policy,
        intent: Option<&Intent>,
    ) -> Result<Evidence, Error> {
        let jobs = self.daemon.jobs(self.checkout()?)?;
        let mut evidence = Evidence {
            known_review_ids: jobs.iter().map(job_id).collect::<Result<_, _>>()?,
            recovered_intent: None,
            request_receipt: None,
            review: None,
        };
        let Some(intent) = intent else {
            return Ok(evidence);
        };
        let key = Self::key(intent);
        // A never-submitted read-only visit must not create a dispatch tombstone.
        if fs::symlink_metadata(self.state.join(format!("{key}.json")))
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            && fs::symlink_metadata(self.state.join(format!("{key}.lock")))
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            return Ok(evidence);
        }
        let (_lock, fresh, path) = claim(&self.state, &key)?;
        let Some(mut saved) = read_submission(&path, fresh)? else {
            return Ok(evidence);
        };
        if saved.snapshot != self.request.snapshot
            || saved.git_ref != self.request.git_ref
            || !saved.intent.matches(candidate, policy)
            || saved.intent.id != intent.id
            || saved.diff_digest != self.request.diff.as_ref().map(|d| hex_digest(d.as_bytes()))
        {
            return Err(Error("daemon intent identity mismatch".into()));
        }
        verify_comparison(&self.request.git_ref, candidate, self.request.scope)?;
        let matching: Vec<_> = jobs
            .iter()
            .filter(|job| {
                self.matches(job) && job_id(job).is_ok_and(|id| !saved.baseline.contains(&id))
            })
            .collect();
        let job = if let Some(existing) = &saved.job {
            let id = job_id(existing)?;
            matching
                .iter()
                .find(|job| job_id(job).ok().as_ref() == Some(&id))
                .copied()
                .ok_or_else(|| Error("persisted daemon job disappeared".into()))?
        } else if matching.len() == 1 {
            matching[0]
        } else if matching.is_empty() {
            return Ok(evidence);
        } else {
            return Err(Error(
                "ambiguous daemon enqueue; multiple jobs match the immutable snapshot".into(),
            ));
        };
        let job = self.detail(job)?;
        let request_receipt = receipt(&job)?;
        if saved
            .job
            .as_ref()
            .is_some_and(|old| old["uuid"] != job["uuid"])
        {
            return Err(Error("job UUID changed".into()));
        }
        saved.job = Some(job.clone());
        super::engine::save(&path, &saved)?;
        evidence.request_receipt = Some(request_receipt);
        match job["status"].as_str() {
            Some("queued" | "running") => {}
            Some("done") => {
                let raw = self.daemon.request(
                    "GET",
                    &format!("/api/review?job_id={}", job_id(&job)?),
                    None,
                )?;
                evidence.review = Some(normalize_roborev_local(
                    &raw,
                    &job,
                    candidate,
                    policy,
                    self.request.expected_files,
                    self.request.scope,
                )?);
            }
            _ => {
                return Err(Error(
                    "daemon job failed, interrupted or returned an unknown execution state".into(),
                ));
            }
        }
        Ok(evidence)
    }
    fn submit(&mut self, intent: &Intent) -> Result<String, Error> {
        verify_comparison(&self.request.git_ref, &intent.candidate, self.request.scope)?;
        let (_lock, fresh, path) = claim(&self.state, &Self::key(intent))?;
        if read_submission(&path, fresh)?.is_some() {
            return Err(Error(
                "enqueue already attempted; reconcile instead of replaying".into(),
            ));
        }
        let baseline = self
            .daemon
            .jobs(self.checkout()?)?
            .iter()
            .map(job_id)
            .collect::<Result<_, _>>()?;
        let mut saved = Submission {
            intent: intent.clone(),
            snapshot: self.request.snapshot.clone(),
            git_ref: self.request.git_ref.clone(),
            baseline,
            job: None,
            dispatching: true,
            diff_digest: self.request.diff.as_ref().map(|d| hex_digest(d.as_bytes())),
        };
        super::engine::save(&path, &saved)?;
        let mut body = json!({"repo_path": self.request.snapshot, "git_ref": self.request.git_ref,
            "agent": self.request.agent, "agentic": false, "review_type": "default", "panel": "none", "min_severity": "low"});
        if let Some(diff) = &self.request.diff {
            body["diff_content"] = Value::String(diff.clone());
        }
        let job = self.daemon.request("POST", "/api/enqueue", Some(&body))?;
        if !self.matches(&job) || !self.matches_diff(&job) {
            return Err(Error("enqueue returned a mismatched job".into()));
        }
        let result = receipt(&job)?;
        saved.job = Some(job);
        saved.dispatching = false;
        super::engine::save(&path, &saved)?;
        Ok(result)
    }
}

/// Normalize complete direct-daemon output, retaining every severity and exact
/// job correlation. Local advisory scopes do not attest the candidate's PR base.
pub fn normalize_roborev_local(
    raw: &Value,
    job: &Value,
    candidate: &Candidate,
    policy: &Policy,
    expected_files: usize,
    scope: RoborevLocalScope,
) -> Result<Review, Error> {
    if raw["job_id"] != job["id"]
        || raw.pointer("/job/uuid") != job.get("uuid")
        || raw.pointer("/job/git_ref") != job.get("git_ref")
    {
        return Err(Error("review/job correlation mismatch".into()));
    }
    let git_ref = job["git_ref"]
        .as_str()
        .ok_or_else(|| Error("persisted comparison missing".into()))?;
    verify_comparison(git_ref, candidate, scope)?;
    let document = raw
        .get("structured_output")
        .ok_or_else(|| Error("canonical review output missing".into()))?;
    if document["schema_version"].as_u64() != Some(2)
        || !document["summary"]
            .as_str()
            .is_some_and(|s| !s.trim().is_empty())
        || !matches!(document["verdict"].as_str(), Some("pass" | "fail"))
    {
        return Err(Error("malformed or legacy review content".into()));
    }
    let findings = document["findings"]
        .as_array()
        .ok_or_else(|| Error("missing findings array".into()))?;
    let review_id = raw["id"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or_else(|| Error("review ID missing".into()))?
        .to_string();
    let mut normalized = Vec::new();
    for (index, finding) in findings.iter().enumerate() {
        let problem = finding["problem"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| Error("finding problem missing".into()))?;
        let fix = finding["fix"]
            .as_str()
            .ok_or_else(|| Error("finding fix missing".into()))?;
        let severity = finding["severity"]
            .as_str()
            .ok_or_else(|| Error("finding severity missing".into()))?;
        if !matches!(severity, "critical" | "high" | "medium" | "low" | "info") {
            return Err(Error("unknown finding severity".into()));
        }
        normalized.push(Finding {
            id: format!(
                "{review_id}:{index}:{}",
                hex_digest(&serde_json::to_vec(finding)?)
            ),
            body: format!("{problem}\n\nSuggested fix: {fix}"),
            path: finding["location"].as_str().map(str::to_owned),
            line: None,
            severity: Some(severity.into()),
            provider_addressed: false,
            correlated: true,
            url: format!("roborev:review:{review_id}"),
        });
    }
    if normalized.is_empty() && document["verdict"].as_str() != Some("pass") {
        return Err(Error("failed verdict with no explanatory findings".into()));
    }
    Ok(Review {
        id: review_id.clone(),
        provider_job_id: Some(job_id(job)?),
        provider_review_id: Some(review_id.clone()),
        provider: "roborev".into(),
        reviewed_head: Some(candidate.head.clone()),
        requested_base: Some(candidate.base.clone()),
        reviewed_base: git_ref.split_once("..").map(|(base, _)| base.to_owned()),
        policy_digest: policy.digest()?,
        completed: true,
        complete_findings: raw
            .pointer("/file_coverage/excluded")
            .and_then(Value::as_u64)
            == Some(0)
            && raw
                .pointer("/file_coverage/reviewed")
                .and_then(Value::as_u64)
                == Some(expected_files as u64),
        findings: normalized,
        url: format!("roborev:review:{review_id}"),
    })
}
