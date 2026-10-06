//! Shared Clap surface for standalone toolbelt and direct Cargo consumers.
use super::{
    Candidate, Error, Forge, GitHub, Outcome, Policy, Verdict, ensure_once, github_provider,
    now_seconds,
};
use clap::{Args, Subcommand};
use std::{
    io::Write,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

/// Routine agent review operations.
#[derive(Debug, Subcommand)]
pub enum ReviewCommand {
    /// Request or resume review, collect findings and wait for current CI
    Ensure(CommonArgs),
    /// Evaluate existing evidence without triggering a review
    Gate(GateArgs),
    /// Record an exact-finding false-positive disposition with source evidence
    Disposition(DispositionArgs),
    /// Inspect the implemented provider/transport and authentication route
    Doctor,
}

/// Explicit configuration overrides; routine calls infer these from user config.
#[derive(Clone, Debug, Args)]
pub struct CommonArgs {
    /// PR URL; otherwise resolve exactly one PR for the current GitHub branch
    #[arg(long)]
    pub pr: Option<String>,
    /// Explicit consumer-owned JSON/Pkl policy (or CANIX_REVIEW_POLICY)
    #[arg(long)]
    pub policy: Option<PathBuf>,
    /// Durable user-owned state directory
    #[arg(long)]
    pub state_dir: Option<PathBuf>,
    /// Repository used for PR discovery
    #[arg(long, default_value = ".")]
    pub repo: PathBuf,
    /// Bound observation and waiting; zero performs one observation
    #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u64).range(0..=3600))]
    pub timeout_seconds: u64,
    /// Require the exact source revision selected by a CI event or caller
    #[arg(long)]
    pub expected_head: Option<String>,
}

/// Existing-evidence gate, including trusted CI check publication.
#[derive(Debug, Args)]
pub struct GateArgs {
    /// Candidate and policy selection.
    #[command(flatten)]
    pub common: CommonArgs,
    /// Publish the review-only check; required CI is enforced independently
    #[arg(long, requires = "details_url")]
    pub publish_check: bool,
    /// Trusted coordinator run permalink
    #[arg(long, requires = "publish_check")]
    pub details_url: Option<String>,
}

/// An evidence-backed disposition is public and revision-bound.
#[derive(Debug, Args)]
pub struct DispositionArgs {
    /// Candidate selection.
    #[command(flatten)]
    pub common: CommonArgs,
    /// Exact finding ID returned by ensure
    #[arg(long)]
    pub finding: String,
    /// Source-grounded explanation of the false positive
    #[arg(long)]
    pub reason: String,
    /// Source/test permalink or precise evidence reference
    #[arg(long)]
    pub evidence: String,
}

/// Merging is a separate explicitly requested effect.
#[derive(Debug, Args)]
pub struct MergeArgs {
    /// Candidate and policy selection.
    #[command(flatten)]
    pub common: CommonArgs,
    /// Explicitly require current provider review and its dedicated policy check
    #[arg(long)]
    pub require_review: bool,
    /// Apply the merge after fresh CI and native protection checks
    #[arg(long)]
    pub apply: bool,
}

/// Resolve credentials through environment or the existing GitHub auth owner.
/// The auth frontend is used only to retrieve credentials; REST owns operations.
pub fn github_from_environment() -> Result<GitHub, Error> {
    let token = std::env::var("GH_TOKEN")
        .or_else(|_| std::env::var("GITHUB_TOKEN"))
        .ok();
    let token = match token {
        Some(token) => token,
        None => {
            let output = Command::new("gh").args(["auth", "token", "--hostname", "github.com"]).output()
                .map_err(|_| Error("GitHub authentication missing: provision GH_TOKEN/GITHUB_TOKEN or gh auth login".into()))?;
            if !output.status.success() {
                return Err(Error(
                    "GitHub authentication missing: run gh auth login".into(),
                ));
            }
            let mut bytes = zeroize::Zeroizing::new(output.stdout);
            let token = std::str::from_utf8(&bytes)
                .map_err(|_| Error("GitHub credential is not UTF-8".into()))?
                .trim()
                .to_owned();
            bytes.clear();
            token
        }
    };
    GitHub::new(token)
}

/// Load user/deployment policy. Repository source branches never supply defaults.
pub fn policy_from_environment(explicit: Option<PathBuf>) -> Result<Policy, Error> {
    if let Some(path) =
        explicit.or_else(|| std::env::var_os("CANIX_REVIEW_POLICY").map(PathBuf::from))
    {
        return Policy::load(&path);
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or_else(|| Error("HOME or XDG_CONFIG_HOME is required for policy discovery".into()))?;
    let path = config.join("canix-toolbelt/review.pkl");
    if path.exists() {
        Policy::load(&path)
    } else {
        Err(Error("explicit consumer-owned review policy is required; configure CANIX_REVIEW_POLICY or canix-toolbelt/review.pkl".into()))
    }
}

fn state_dir(explicit: Option<PathBuf>) -> Result<PathBuf, Error> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    let root = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .ok_or_else(|| {
            Error("HOME or XDG_STATE_HOME is required for durable review state".into())
        })?;
    Ok(root.join("canix-toolbelt/review"))
}

fn resolve_pr(args: &CommonArgs, github: &GitHub) -> Result<String, Error> {
    if let Some(pr) = &args.pr {
        return Ok(pr.clone());
    }
    let git = |arguments: &[&str]| -> Result<String, Error> {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(&args.repo)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()?;
        if !output.status.success() {
            return Err(Error(
                "GitHub PR discovery requires a Git checkout with origin and an active branch"
                    .into(),
            ));
        }
        String::from_utf8(output.stdout)
            .map(|s| s.trim().to_owned())
            .map_err(|_| Error("Git discovery output is not UTF-8".into()))
    };
    let origin = git(&["remote", "get-url", "origin"])?;
    let repository = origin.strip_prefix("git@github.com:").or_else(|| origin.strip_prefix("https://github.com/"))
        .ok_or_else(|| Error("unsupported forge/provider pairing; supply a GitHub PR URL or enroll a supported provider".into()))?.trim_end_matches(".git");
    let branch = git(&["branch", "--show-current"])?;
    if branch.is_empty() {
        return Err(Error(
            "detached checkout requires an explicit --pr URL".into(),
        ));
    }
    github.discover(repository, &branch)
}

struct ReviewOnly<'a>(&'a mut GitHub);

fn poll_delay(retry_at: Option<u64>, now: u64, remaining: Duration) -> Option<Duration> {
    let delay = match retry_at {
        Some(at) => Duration::from_secs(at.saturating_sub(now).max(1)),
        None => Duration::from_secs(30)
            .min(remaining / 2)
            .max(Duration::from_secs(1)),
    };
    (delay < remaining).then_some(delay)
}
impl Forge for ReviewOnly<'_> {
    fn candidate(&mut self, url: &str) -> Result<Candidate, Error> {
        self.0.candidate(url)
    }
    fn checks(&mut self, _: &Candidate, _: &Policy) -> Result<Vec<String>, Error> {
        Ok(vec![])
    }
    fn next_poll_at(&self) -> Option<u64> {
        self.0.next_poll_at()
    }
}

/// Execute the routine flow. Returns structured evidence without merging.
pub fn ensure(args: CommonArgs, allow_submit: bool, review_only: bool) -> Result<Outcome, Error> {
    let policy = policy_from_environment(args.policy.clone())?;
    let mut github = github_from_environment()?.with_deadline(Duration::from_secs(
        if args.timeout_seconds == 0 {
            60
        } else {
            args.timeout_seconds
        },
    ));
    let pr = resolve_pr(&args, &github)?;
    let path = state_dir(args.state_dir.clone())?;
    let mut provider = github_provider(github.clone(), &policy)?;
    if let Some(expected) = &args.expected_head {
        if &github.candidate(&pr)?.head != expected {
            return Err(Error("candidate is stale relative to the caller's expected head; no review was submitted".into()));
        }
    }
    let deadline = Instant::now() + Duration::from_secs(args.timeout_seconds);
    loop {
        let mut result = if review_only {
            ensure_once(
                &mut ReviewOnly(&mut github),
                provider.as_mut(),
                &policy,
                &pr,
                &path,
                allow_submit,
            )?
        } else {
            ensure_once(
                &mut github,
                provider.as_mut(),
                &policy,
                &pr,
                &path,
                allow_submit,
            )?
        };
        if args
            .expected_head
            .as_ref()
            .is_some_and(|head| head != &result.candidate.head)
        {
            result.verdict = Verdict::Stale;
            result
                .blockers
                .push("candidate does not match the caller's expected head".into());
            result.next_action = "Reconcile the event/candidate revision before continuing.".into();
        }
        if review_only {
            result.next_action = "This is a review-only result. Required CI and merge authorization remain independently enforced.".into();
        }
        let waitable = result.verdict == Verdict::Pending
            || result.next_poll_at.is_some()
            || (result.verdict == Verdict::Blocked
                && result.review.is_some()
                && result.blockers.iter().all(|b| b.contains("CI")));
        if !waitable || Instant::now() >= deadline {
            return Ok(result);
        }
        let Some(delay) = poll_delay(
            result.next_poll_at,
            now_seconds(),
            deadline.saturating_duration_since(Instant::now()),
        ) else {
            return Ok(result);
        };
        std::thread::sleep(delay);
    }
}

fn print(value: &impl serde::Serialize) -> Result<(), Error> {
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, value)?;
    writeln!(stdout)?;
    Ok(())
}

/// Execute a review subcommand and return its stable exit code.
pub fn run(command: ReviewCommand) -> Result<u8, Error> {
    match command {
        ReviewCommand::Doctor => {
            let policy = policy_from_environment(None)?;
            let github = github_from_environment()?;
            github.authenticate()?;
            print(&github_provider(github, &policy)?.capabilities())?;
            Ok(0)
        }
        ReviewCommand::Ensure(args) => {
            let result = ensure(args, true, false)?;
            print(&result)?;
            Ok(result.verdict.exit_code())
        }
        ReviewCommand::Gate(args) => {
            let policy = policy_from_environment(args.common.policy.clone())?;
            if args.publish_check {
                // Invalidate older passing evidence before fallible collection.
                let mut github = github_from_environment()?;
                let pr = resolve_pr(&args.common, &github)?;
                let candidate = github.candidate(&pr)?;
                if args
                    .common
                    .expected_head
                    .as_ref()
                    .is_some_and(|head| head != &candidate.head)
                {
                    return Err(Error(
                        "CI event head is stale; no check was published for a different candidate"
                            .into(),
                    ));
                }
                github.publish_check(
                    &candidate,
                    &policy,
                    false,
                    args.details_url.as_deref().ok_or_else(|| {
                        Error("check publication requires a coordinator permalink".into())
                    })?,
                )?;
            }
            let result = ensure(args.common, false, args.publish_check)?;
            print(&result)?;
            if args.publish_check && result.verdict != Verdict::Stale {
                let details = args.details_url.ok_or_else(|| {
                    Error("check publication requires a coordinator permalink".into())
                })?;
                github_from_environment()?.publish_check(
                    &result.candidate,
                    &policy,
                    result.verdict == Verdict::Ready,
                    &details,
                )?;
            }
            Ok(result.verdict.exit_code())
        }
        ReviewCommand::Disposition(args) => {
            let policy = policy_from_environment(args.common.policy.clone())?;
            let mut common = args.common;
            common.timeout_seconds = 0;
            let result = ensure(common, false, false)?;
            let review = result
                .review
                .ok_or_else(|| Error("no current review is available for disposition".into()))?;
            let finding = review
                .findings
                .iter()
                .find(|f| f.id == args.finding)
                .ok_or_else(|| Error("finding is not in the current review".into()))?;
            github_from_environment()?.disposition(
                &result.candidate,
                &policy,
                &review,
                finding,
                &args.reason,
                &args.evidence,
            )?;
            print(
                &serde_json::json!({"verdict":"pending", "nextAction":"Rerun ensure to verify the authorized disposition and current evidence."}),
            )?;
            Ok(Verdict::Pending.exit_code())
        }
    }
}

/// Execute or preview an authorized merge after fresh gate evaluation.
pub fn merge(args: MergeArgs) -> Result<u8, Error> {
    if !args.require_review {
        if args.common.policy.is_some() || args.common.state_dir.is_some() {
            return Err(Error(
                "--policy and --state-dir require --require-review for merge".into(),
            ));
        }
        let mut github = github_from_environment()?.with_deadline(Duration::from_secs(
            if args.common.timeout_seconds == 0 {
                60
            } else {
                args.common.timeout_seconds
            },
        ));
        let pr = resolve_pr(&args.common, &github)?;
        let candidate = github.candidate(&pr)?;
        if args
            .common
            .expected_head
            .as_ref()
            .is_some_and(|head| head != &candidate.head)
        {
            return Err(Error(
                "merge candidate does not match the caller's expected head".into(),
            ));
        }
        print(&github.merge_native(&candidate, args.apply)?)?;
        return Ok(0);
    }
    let policy = policy_from_environment(args.common.policy.clone())?;
    let result = ensure(args.common, true, false)?;
    print(&result)?;
    if result.verdict != Verdict::Ready {
        return Ok(result.verdict.exit_code());
    }
    if args.apply {
        print(&github_from_environment()?.merge(&result.candidate, &policy)?)?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::poll_delay;
    use std::time::Duration;

    #[test]
    fn short_deadlines_leave_time_for_another_observation() {
        for seconds in 15..30 {
            let remaining = Duration::from_secs(seconds);
            let delay = poll_delay(None, 100, remaining).unwrap();
            assert!(delay < remaining);
            assert_eq!(delay, remaining / 2);
        }
        assert_eq!(
            poll_delay(None, 100, Duration::from_secs(600)),
            Some(Duration::from_secs(30))
        );
    }

    #[test]
    fn provider_retry_times_are_never_shortened_for_a_deadline() {
        assert_eq!(poll_delay(Some(140), 100, Duration::from_secs(20)), None);
        assert_eq!(
            poll_delay(Some(115), 100, Duration::from_secs(20)),
            Some(Duration::from_secs(15))
        );
        assert_eq!(poll_delay(None, 100, Duration::ZERO), None);
    }
}
