use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// A diagnostic which never contains credentials or remote response bodies.
#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}

/// Pull-request coordinates. Only explicitly implemented forges are accepted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    /// Canonical forge host.
    pub host: String,
    /// Owner and repository, separated by `/`.
    pub repository: String,
    /// Forge-assigned request number.
    pub number: u64,
}

impl PullRequest {
    /// Parse an HTTPS GitHub PR URL without permitting credential or path injection.
    pub fn parse(input: &str) -> Result<Self, Error> {
        let url = url::Url::parse(input)
            .map_err(|_| Error("expected an HTTPS pull-request URL".into()))?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.host_str() != Some("github.com")
            || url.query().is_some()
        {
            return Err(Error(
                "unsupported forge/provider pairing; GitHub + Greptile is implemented".into(),
            ));
        }
        let parts: Vec<_> = url
            .path()
            .trim_end_matches('/')
            .split('/')
            .skip(1)
            .collect();
        if parts.len() != 4
            || parts[2] != "pull"
            || !parts[..2].iter().all(|p| {
                !p.is_empty()
                    && *p != "."
                    && *p != ".."
                    && p.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            })
        {
            return Err(Error(
                "expected https://github.com/OWNER/REPO/pull/NUMBER".into(),
            ));
        }
        let number = parts[3]
            .parse::<u64>()
            .map_err(|_| Error("invalid pull-request number".into()))?;
        if number == 0 {
            return Err(Error("pull-request number must be positive".into()));
        }
        Ok(Self {
            host: "github.com".into(),
            repository: format!("{}/{}", parts[0], parts[1]),
            number,
        })
    }

    /// Canonical URL, including no fragment or user-controlled query.
    pub fn url(&self) -> String {
        format!(
            "https://{}/{}/pull/{}",
            self.host, self.repository, self.number
        )
    }
}

/// The immutable comparison selected for one request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Candidate {
    /// Canonical PR/MR URL.
    pub url: String,
    /// Source repository, including fork identity.
    pub source_repository: String,
    /// Source branch name.
    pub source_branch: String,
    /// Target branch name.
    pub target_branch: String,
    /// Expected source commit.
    pub head: String,
    /// Observed target commit at dispatch; not provider base attestation.
    pub base: String,
    /// Draft requests may be reviewed, but not merged.
    pub draft: bool,
    /// Whether the request is currently open.
    pub open: bool,
}

impl Candidate {
    /// Validate immutable identity before dispatch or persistence.
    pub fn validate(&self) -> Result<(), Error> {
        let url = url::Url::parse(&self.url)
            .map_err(|_| Error("invalid canonical candidate URL".into()))?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error(
                "candidate requires a canonical HTTPS URL without credentials, query or fragment"
                    .into(),
            ));
        }
        for sha in [&self.head, &self.base] {
            if ![40, 64].contains(&sha.len()) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Error(
                    "candidate requires full head and base revisions".into(),
                ));
            }
        }
        if self.source_repository.is_empty()
            || self.source_branch.is_empty()
            || self.target_branch.is_empty()
        {
            return Err(Error(
                "candidate requires source repository and both branches".into(),
            ));
        }
        Ok(())
    }
}

/// A required CI context, optionally restricted to its producing GitHub App.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckRequirement {
    /// Exact check/context name.
    pub name: String,
    /// Expected App identity; `None` accepts a legacy status context.
    pub app_id: Option<u64>,
}

/// Consumer policy. Missing or unknown fields cannot silently weaken the contract.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    /// Wire contract version.
    pub schema_version: u32,
    /// Review provider identity.
    pub provider: String,
    /// Explicit supported transport.
    pub transport: String,
    /// Immutable forge account ID of the trusted reviewer.
    pub reviewer_id: u64,
    /// Additional required CI contexts; native branch protection is also honored.
    pub required_checks: Vec<CheckRequirement>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            schema_version: 1,
            provider: "greptile".into(),
            transport: "github-comment".into(),
            reviewer_id: 165735046,
            required_checks: vec![],
        }
    }
}

impl Policy {
    /// Load explicit consumer-owned JSON or Pkl policy; never discover policy
    /// from the untrusted source branch of the request being reviewed.
    pub fn load(path: &std::path::Path) -> Result<Self, Error> {
        let policy: Self = if path.extension().is_some_and(|s| s == "pkl") {
            fleetix::pkl::load_sync(path)
                .map_err(|e| Error(format!("review policy evaluation failed: {e}")))?
        } else {
            serde_json::from_slice(&std::fs::read(path)?)?
        };
        policy.validate()?;
        Ok(policy)
    }
    /// Reject unsupported policy rather than changing provider or transport.
    pub fn validate(&self) -> Result<(), Error> {
        let identity = |s: &str| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b))
        };
        if self.schema_version != 1
            || !identity(&self.provider)
            || !identity(&self.transport)
            || self.reviewer_id == 0
        {
            return Err(Error(
                "invalid review policy version or provider/transport identity".into(),
            ));
        }
        if self
            .required_checks
            .iter()
            .any(|c| c.name.trim().is_empty())
        {
            return Err(Error("required check names cannot be empty".into()));
        }
        Ok(())
    }

    /// Stable policy identity, retained by request and review receipts.
    pub fn digest(&self) -> Result<String, Error> {
        self.validate()?;
        Ok(hex_digest(&serde_json::to_vec(self)?))
    }

    /// Evaluate review provenance and findings; CI and merge eligibility are separate.
    pub fn evaluate(&self, candidate: &Candidate, review: &Review) -> Verdict {
        if self.validate().is_err()
            || candidate.validate().is_err()
            || !candidate.open
            || review.provider != self.provider
        {
            return Verdict::Blocked;
        }
        if !review.completed {
            return Verdict::Pending;
        }
        let Some(head) = &review.reviewed_head else {
            return Verdict::Blocked;
        };
        let Some(base) = &review.requested_base else {
            return Verdict::Blocked;
        };
        if head != &candidate.head
            || base != &candidate.base
            || self.digest().ok().as_ref() != Some(&review.policy_digest)
        {
            return Verdict::Stale;
        }
        if !review.complete_findings {
            return Verdict::Blocked;
        }
        if !review.findings.is_empty() {
            return Verdict::Findings;
        }
        Verdict::Ready
    }
}

/// One normalized finding. Addressed metadata does not establish a fix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Stable provider/forge comment identity.
    pub id: String,
    /// Untrusted original finding text.
    pub body: String,
    /// Original file location, if supplied.
    pub path: Option<String>,
    /// Current or original line, if supplied.
    pub line: Option<u64>,
    /// Provider severity; `None` remains a blocking unknown.
    pub severity: Option<String>,
    /// Provider-reported addressed status, diagnostic only.
    pub provider_addressed: bool,
    /// Source permalink.
    pub url: String,
}

/// Review evidence associated with a specific provider run/forge review.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    /// Provider run or parent forge review ID; never an inline remapped commit.
    pub id: String,
    /// Provider name.
    pub provider: String,
    /// Proven reviewed source commit.
    pub reviewed_head: Option<String>,
    /// Base observed at request dispatch and rechecked during collection.
    pub requested_base: Option<String>,
    /// Policy used to request the review.
    pub policy_digest: String,
    /// Whether the provider has submitted its completed review.
    pub completed: bool,
    /// Whether every page of this review's findings was collected.
    pub complete_findings: bool,
    /// Remaining blocking findings after validated dispositions.
    pub findings: Vec<Finding>,
    /// Source review permalink.
    pub url: String,
}

/// Stable machine-readable next-action state. Only `Ready` exits successfully.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Complete current review with no unresolved findings.
    Ready,
    /// Findings require source triage.
    Findings,
    /// Work is still in flight.
    Pending,
    /// Missing, unsupported or incomplete evidence.
    Blocked,
    /// Revision or policy movement invalidated evidence.
    Stale,
}

impl Verdict {
    /// Stable CLI exit code for automation.
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::Findings => 20,
            Self::Pending => 21,
            Self::Blocked => 22,
            Self::Stale => 23,
        }
    }
}

pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
