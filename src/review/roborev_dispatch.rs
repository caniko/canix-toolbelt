//! Write-ahead, request-scoped roborev dispatch and ambiguous-enqueue recovery.
use super::{Error, Intent, Policy, RoborevReceipt, model::hex_digest};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

/// Consumer-prepared immutable comparison, in a checkout unique to this request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoborevDispatch {
    /// Authorized forge intent, obtained from trusted policy and fresh forge facts.
    pub intent: Intent,
    /// Absolute request-specific checkout, not a shared worktree or personal clone.
    pub checkout: String,
    /// Explicit approved adapter selection; no model/provider override is supplied.
    pub agent: String,
}

/// Actual daemon/checkout boundary. Implementations must perform bounded I/O,
/// return all pages, and never substitute Markdown for persisted review content.
pub trait RoborevRunner {
    /// Verify both full commits in the immutable request checkout and the trusted
    /// worker policy. PR-controlled settings/hooks must not alter execution.
    fn verify_checkout(&mut self, dispatch: &RoborevDispatch) -> Result<(), Error>;
    /// All jobs for the request-exclusive checkout, including failed jobs. This
    /// must not hide jobs by range, status, severity, or default pagination.
    fn jobs(&mut self, dispatch: &RoborevDispatch) -> Result<Vec<Value>, Error>;
    /// Submit the full base..head range once, no panel, default review type and
    /// low/unfiltered findings. Any error is an UNKNOWN mutation outcome.
    fn enqueue(&mut self, dispatch: &RoborevDispatch) -> Result<u64, Error>;
    /// Complete persisted review with its job metadata and canonical document.
    fn saved_review(&mut self, job_id: u64) -> Result<Value, Error>;
}

impl<T: RoborevRunner + ?Sized> RoborevRunner for &mut T {
    fn verify_checkout(&mut self, dispatch: &RoborevDispatch) -> Result<(), Error> {
        (**self).verify_checkout(dispatch)
    }
    fn jobs(&mut self, dispatch: &RoborevDispatch) -> Result<Vec<Value>, Error> {
        (**self).jobs(dispatch)
    }
    fn enqueue(&mut self, dispatch: &RoborevDispatch) -> Result<u64, Error> {
        (**self).enqueue(dispatch)
    }
    fn saved_review(&mut self, job_id: u64) -> Result<Value, Error> {
        (**self).saved_review(job_id)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    schema_version: u32,
    dispatch: RoborevDispatch,
    // None means UNKNOWN, never permission to retry an enqueue.
    job_id: Option<u64>,
}

fn open_private(path: &Path, create_new: bool) -> Result<fs::File, std::io::Error> {
    let mut options = fs::OpenOptions::new();
    options
        .read(true)
        .write(create_new)
        .create_new(create_new)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    options.open(path)
}

pub(super) fn claim(state_dir: &Path, key: &str) -> Result<(fs::File, bool, PathBuf), Error> {
    let mut directory = fs::DirBuilder::new();
    directory.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        directory.mode(0o700);
        directory.create(state_dir)?;
        if fs::symlink_metadata(state_dir)?.mode() & 0o077 != 0 {
            return Err(Error("roborev state must have private permissions".into()));
        }
    }
    #[cfg(not(unix))]
    directory.create(state_dir)?;
    if !fs::symlink_metadata(state_dir)?.file_type().is_dir() {
        return Err(Error("roborev state must be a private directory".into()));
    }
    let lock_path = state_dir.join(format!("{key}.lock"));
    let (lock, fresh) = match open_private(&lock_path, true) {
        Ok(file) => (file, true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            (open_private(&lock_path, false)?, false)
        }
        Err(error) => return Err(error.into()),
    };
    lock.try_lock_exclusive()
        .map_err(|_| Error("another worker owns this roborev request".into()))?;
    Ok((lock, fresh, state_dir.join(format!("{key}.json"))))
}

pub(super) fn read_ledger<T: serde::de::DeserializeOwned>(
    path: &Path,
    fresh: bool,
) -> Result<Option<T>, Error> {
    read_ledger_with_limit(path, fresh, 1_048_576)
}

pub(super) fn read_ledger_with_limit<T: serde::de::DeserializeOwned>(
    path: &Path,
    fresh: bool,
    limit: u64,
) -> Result<Option<T>, Error> {
    match open_private(path, false) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(limit + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > limit {
                return Err(Error("oversized roborev ledger".into()));
            }
            Ok(Some(serde_json::from_slice::<T>(&bytes)?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && fresh => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(Error(
            "roborev ledger is missing behind a claimed request; no mutation replay is permitted"
                .into(),
        )),
        Err(error) => Err(error.into()),
    }
}

/// Reconcile one authorized request without sleeping, forge writes or replaying
/// an ambiguous enqueue. The consumer must revalidate the open comparison and
/// authorized request immediately before publishing the returned complete receipt.
/// A pending job returns None; a terminal failure returns explicit null evidence.
pub fn dispatch_roborev_once<R: RoborevRunner>(
    dispatch: &RoborevDispatch,
    policy: &Policy,
    state_dir: &Path,
    runner: &mut R,
) -> Result<Option<RoborevReceipt>, Error> {
    policy.validate()?;
    policy.policy_app_id()?;
    dispatch.intent.candidate.validate()?;
    if policy.provider != "roborev"
        || policy.transport != "github-receipt-v1"
        || !dispatch.intent.matches(&dispatch.intent.candidate, policy)
        || !dispatch.intent.candidate.open
        || dispatch.intent.id.is_empty()
        || !matches!(
            dispatch.agent.as_str(),
            "opencode" | "codex" | "claude-code"
        )
        || !Path::new(&dispatch.checkout).is_absolute()
        || dispatch.checkout.chars().any(char::is_control)
        || dispatch
            .checkout
            .split('/')
            .any(|part| part == "." || part == "..")
    {
        return Err(Error(
            "invalid authorized roborev dispatch or worker selection".into(),
        ));
    }
    let key =
        hex_digest(format!("{}:{}", dispatch.intent.candidate.url, dispatch.intent.id).as_bytes());
    // The persistent anchor is also a dispatch tombstone. A missing ledger
    // behind an existing anchor cannot be treated as a never-dispatched request.
    let (_lock, fresh, path) = claim(state_dir, &key)?;
    let mut ledger = read_ledger::<Ledger>(&path, fresh)?;
    if let Some(old) = &ledger {
        if old.schema_version != 1
            || serde_json::to_value(&old.dispatch)? != serde_json::to_value(dispatch)?
        {
            return Err(Error(
                "roborev request identity was reused with different dispatch content".into(),
            ));
        }
    }
    runner.verify_checkout(dispatch)?;
    let mut jobs = runner.jobs(dispatch)?;
    if ledger.is_none() {
        if !jobs.is_empty() {
            return Err(Error("request checkout already has roborev history; no job may be adopted before dispatch".into()));
        }
        let mut new = Ledger {
            schema_version: 1,
            dispatch: dispatch.clone(),
            job_id: None,
        };
        super::engine::save(&path, &new)?;
        // UNKNOWN is durable before this effect, including crashes and timeouts.
        let id = runner.enqueue(dispatch)?;
        if id == 0 {
            return Err(Error(
                "roborev enqueue did not return a persisted job identity".into(),
            ));
        }
        new.job_id = Some(id);
        super::engine::save(&path, &new)?;
        ledger = Some(new);
        jobs = runner.jobs(dispatch)?;
    }
    if jobs.len() != 1 {
        return Err(Error("roborev dispatch is unknown or ambiguous; reconcile persisted jobs without another enqueue".into()));
    }
    let job = &jobs[0];
    let id = job["id"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or_else(|| Error("missing persisted roborev job identity".into()))?;
    let range = format!(
        "{}..{}",
        dispatch.intent.candidate.base, dispatch.intent.candidate.head
    );
    if job["repo_path"].as_str() != Some(&dispatch.checkout)
        || job["git_ref"].as_str() != Some(&range)
        || job["agent"].as_str() != Some(&dispatch.agent)
        || job["job_type"].as_str() != Some("range")
        || job["agentic"].as_bool() != Some(false)
        || job["prompt_prebuilt"].as_bool() != Some(false)
        || !matches!(job["min_severity"].as_str(), Some("" | "low"))
        || ledger
            .as_ref()
            .and_then(|old| old.job_id)
            .is_some_and(|old| old != id)
    {
        return Err(Error(
            "persisted job does not match the exclusive frozen roborev dispatch".into(),
        ));
    }
    let mut ledger = ledger.ok_or_else(|| Error("missing write-ahead roborev intent".into()))?;
    ledger.job_id = Some(id);
    super::engine::save(&path, &ledger)?;
    match job["status"].as_str() {
        Some("queued" | "running") => Ok(None),
        Some("done") => RoborevReceipt::from_saved_review(
            &dispatch.intent,
            policy,
            id,
            &dispatch.checkout,
            &dispatch.agent,
            &runner.saved_review(id)?,
        )
        .map(Some),
        Some(status @ ("failed" | "skipped" | "cancelled")) => Ok(Some(RoborevReceipt {
            schema_version: 1,
            request_id: dispatch.intent.id.clone(),
            candidate: dispatch.intent.candidate.clone(),
            policy_digest: dispatch.intent.policy_digest.clone(),
            job_id: Some(id),
            review_id: None,
            reviewed_head: dispatch.intent.candidate.head.clone(),
            reviewed_base: dispatch.intent.candidate.base.clone(),
            status: status.into(),
            complete_findings: false,
            document: None,
        })),
        _ => Err(Error(
            "unsupported persisted roborev execution state".into(),
        )),
    }
}
