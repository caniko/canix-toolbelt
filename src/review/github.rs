use super::{
    Candidate, Capabilities, CheckRequirement, Error, Evidence, Finding, Forge, Intent, Policy,
    Provider, PullRequest, Review, model::hex_digest, now_seconds,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const REQUEST_MARKER: &str = "<!-- toolbelt-review-request:v1 ";
const DISPOSITION_MARKER: &str = "<!-- toolbelt-review-disposition:v1 ";

/// Native GitHub REST transport. Credentials never enter persisted receipts.
#[derive(Clone)]
pub struct GitHub {
    agent: ureq::Agent,
    token: Arc<zeroize::Zeroizing<String>>,
    endpoint: String,
    retry_at: Arc<Mutex<Option<u64>>>,
    deadline: Option<Instant>,
}

impl GitHub {
    /// Connect to GitHub Cloud with a consumer-supplied credential.
    pub fn new(token: String) -> Result<Self, Error> {
        if token.trim().is_empty() || token.contains(['\r', '\n']) {
            return Err(Error("a nonempty GitHub credential is required".into()));
        }
        Ok(Self {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(20)))
                .http_status_as_error(false)
                .max_redirects(0)
                .build()
                .new_agent(),
            token: Arc::new(zeroize::Zeroizing::new(token)),
            endpoint: "https://api.github.com".into(),
            retry_at: Arc::new(Mutex::new(None)),
            deadline: None,
        })
    }

    /// Bound the whole collection in addition to each individual HTTP request.
    pub fn with_deadline(mut self, duration: Duration) -> Self {
        self.deadline = Some(Instant::now() + duration);
        self
    }

    /// Verify that the credential can reach the authenticated GitHub API.
    pub fn authenticate(&self) -> Result<(), Error> {
        self.get("/rate_limit").map(|_| ())
    }

    fn request(
        &self,
        method: &str,
        route: &str,
        body: Option<Value>,
    ) -> Result<(u16, Value), Error> {
        if self.deadline.is_some_and(|at| Instant::now() >= at) {
            return Err(Error(
                "review collection deadline reached; resume the same ensure request".into(),
            ));
        }
        if let Ok(mut retry) = self.retry_at.lock() {
            *retry = None;
        }
        let request = ureq::http::Request::builder()
            .method(method)
            .uri(format!("{}{route}", self.endpoint))
            .header("Authorization", format!("Bearer {}", self.token.as_str()))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "canix-toolbelt-review")
            .header("Content-Type", "application/json")
            .body(body.map(|v| v.to_string()).unwrap_or_default())
            .map_err(|_| Error("invalid GitHub request".into()))?;
        let mut response = self.agent.run(request).map_err(|_| {
            Error("GitHub transport failed; remote mutation outcome may be unknown".into())
        })?;
        let status = response.status().as_u16();
        if status == 429
            || (status == 403
                && (response.headers().get("retry-after").is_some()
                    || response
                        .headers()
                        .get("x-ratelimit-remaining")
                        .and_then(|v| v.to_str().ok())
                        == Some("0")))
        {
            let header = |name| {
                response
                    .headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
            };
            let at = header("retry-after")
                .map(|n| now_seconds().saturating_add(n))
                .or_else(|| header("x-ratelimit-reset"))
                .unwrap_or_else(|| now_seconds().saturating_add(60))
                .max(now_seconds().saturating_add(1));
            if let Ok(mut retry) = self.retry_at.lock() {
                *retry = Some(at);
            }
            return Err(Error(format!(
                "GitHub rate-limited this request; next eligible poll is Unix {at}"
            )));
        }
        if status == 404 {
            return Ok((status, Value::Null));
        }
        if !(200..300).contains(&status) {
            return Err(Error(format!(
                "GitHub {method} request failed with HTTP {status}"
            )));
        }
        if status == 204 {
            return Ok((status, Value::Null));
        }
        let bytes = response
            .body_mut()
            .with_config()
            .limit(8 * 1024 * 1024)
            .read_to_vec()
            .map_err(|_| Error("GitHub response exceeded the bound or could not be read".into()))?;
        Ok((status, serde_json::from_slice(&bytes)?))
    }

    fn get(&self, route: &str) -> Result<Value, Error> {
        let (status, value) = self.request("GET", route, None)?;
        if status == 404 {
            return Err(Error(
                "GitHub resource missing or not accessible to this credential".into(),
            ));
        }
        Ok(value)
    }

    fn pages(&self, route: &str, field: Option<&str>) -> Result<Vec<Value>, Error> {
        let mut all = Vec::new();
        for page in 1..=100 {
            let separator = if route.contains('?') { '&' } else { '?' };
            let value = self.get(&format!("{route}{separator}per_page=100&page={page}"))?;
            let items = field
                .map_or(Some(&value), |f| value.get(f))
                .and_then(Value::as_array)
                .ok_or_else(|| Error("GitHub returned an incomplete collection schema".into()))?;
            if items.len() > 100 {
                return Err(Error("GitHub page exceeded its requested bound".into()));
            }
            all.extend(items.iter().cloned());
            if items.len() < 100 {
                if let Some(total) = value.get("total_count").and_then(Value::as_u64) {
                    if total != all.len() as u64 {
                        return Err(Error(
                            "GitHub pagination total changed or collection is incomplete".into(),
                        ));
                    }
                }
                return Ok(all);
            }
        }
        Err(Error(
            "GitHub collection exceeded 100 pages; evidence remains incomplete".into(),
        ))
    }

    fn authorized(
        &self,
        pr: &PullRequest,
        comment: &Value,
        permissions: &mut BTreeMap<String, bool>,
    ) -> Result<bool, Error> {
        let login = text(comment, "/user/login")?;
        if let Some(allowed) = permissions.get(login) {
            return Ok(*allowed);
        }
        if !login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-[]".contains(&b))
        {
            return Ok(false);
        }
        let (status, result) = self.request(
            "GET",
            &format!("/repos/{}/collaborators/{login}/permission", pr.repository),
            None,
        )?;
        if status == 404 {
            permissions.insert(login.into(), false);
            return Ok(false);
        }
        let allowed = matches!(
            result.get("permission").and_then(Value::as_str),
            Some("admin" | "write" | "maintain")
        );
        permissions.insert(login.into(), allowed);
        Ok(allowed)
    }

    /// Resolve exactly one open PR for a branch. Ambiguity remains an explicit blocker.
    pub fn discover(&self, repository: &str, branch: &str) -> Result<String, Error> {
        let coordinate = PullRequest::parse(&format!("https://github.com/{repository}/pull/1"))?;
        let owner = coordinate
            .repository
            .split('/')
            .next()
            .ok_or_else(|| Error("missing owner".into()))?;
        let head: String =
            url::form_urlencoded::byte_serialize(format!("{owner}:{branch}").as_bytes()).collect();
        let prs = self.pages(
            &format!("/repos/{repository}/pulls?state=open&head={head}"),
            None,
        )?;
        if prs.len() != 1 {
            return Err(Error("expected exactly one open PR for this branch; supply --pr URL or publish an authorized candidate PR".into()));
        }
        Ok(text(&prs[0], "/html_url")?.into())
    }

    /// Merge with atomic expected-head protection and strict native branch protection.
    /// A transport error must be reconciled from the PR before any repeat.
    pub fn merge(&mut self, candidate: &Candidate, policy: &Policy) -> Result<Value, Error> {
        candidate.validate()?;
        policy.validate()?;
        let current = self.candidate(&candidate.url)?;
        if !same_comparison(candidate, &current) || !current.open || current.draft {
            return Err(Error(
                "merge candidate moved, closed or remains draft; rerun ensure".into(),
            ));
        }
        let pr = PullRequest::parse(&candidate.url)?;
        let protection = self.get(&format!(
            "/repos/{}/branches/{}/protection",
            pr.repository,
            segment(&candidate.target_branch)
        ))?;
        if protection
            .pointer("/enforce_admins/enabled")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Err(Error(
                "merge protection must apply to administrators as well as contributors".into(),
            ));
        }
        let required = protection
            .pointer("/required_status_checks")
            .ok_or_else(|| Error("merge requires strict native status-check protection".into()))?;
        if required.get("strict").and_then(Value::as_bool) != Some(true) {
            return Err(Error("merge requires strict up-to-date branch protection; merge-queue/ruleset support needs its own adapter".into()));
        }
        let checks = required
            .get("checks")
            .and_then(Value::as_array)
            .ok_or_else(|| Error("protected review-policy check is not configured".into()))?;
        if !checks.iter().any(|c| {
            c.get("context").and_then(Value::as_str) == Some("review-policy")
                && c.get("app_id")
                    .and_then(Value::as_u64)
                    .is_some_and(|id| id > 0 && id != 15368)
        }) {
            return Err(Error("merge requires review-policy bound to a dedicated policy GitHub App; the shared GitHub Actions App cannot distinguish untrusted PR workflows".into()));
        }
        let blockers = self.checks(candidate, policy)?;
        if !blockers.is_empty() {
            return Err(Error(format!("merge CI gate: {}", blockers.join("; "))));
        }
        let fresh = self.candidate(&candidate.url)?;
        if !same_comparison(candidate, &fresh) {
            return Err(Error("comparison changed immediately before merge".into()));
        }
        // The public library merge entry point owns review enforcement too;
        // callers cannot turn an earlier ready receipt into merge authority.
        let evidence = GreptileGitHub::new(self.clone()).inspect(&fresh, policy, None)?;
        if evidence
            .review
            .as_ref()
            .is_none_or(|review| policy.evaluate(&fresh, review) != super::Verdict::Ready)
        {
            return Err(Error(
                "completed current provider review no longer qualifies; rerun ensure".into(),
            ));
        }
        if !same_comparison(candidate, &self.candidate(&candidate.url)?) {
            return Err(Error(
                "comparison changed during final review validation".into(),
            ));
        }
        let (_, value) = self.request(
            "PUT",
            &format!("/repos/{}/pulls/{}/merge", pr.repository, pr.number),
            Some(json!({"sha": candidate.head, "merge_method": "merge"})),
        )?;
        if value.get("merged").and_then(Value::as_bool) != Some(true) {
            return Err(Error(
                "GitHub did not confirm merge; inspect authoritative PR state".into(),
            ));
        }
        Ok(value)
    }

    /// Publish the review-only policy check from a trusted CI coordinator.
    pub fn publish_check(
        &self,
        candidate: &Candidate,
        ready: bool,
        details_url: &str,
    ) -> Result<(), Error> {
        let pr = PullRequest::parse(&candidate.url)?;
        self.request("POST", &format!("/repos/{}/check-runs", pr.repository), Some(json!({
            "name": "review-policy", "head_sha": candidate.head, "status": "completed",
            "conclusion": if ready { "success" } else { "failure" },
            "details_url": details_url,
            "output": {"title": if ready { "Current review qualifies" } else { "Review evidence does not qualify" },
                "summary": "Revision-bound review policy evaluated by canix-toolbelt. Required CI remains independently enforced."}
        })))?;
        Ok(())
    }
}

impl Forge for GitHub {
    fn candidate(&mut self, input: &str) -> Result<Candidate, Error> {
        let pr = PullRequest::parse(input)?;
        let value = self.get(&format!("/repos/{}/pulls/{}", pr.repository, pr.number))?;
        Ok(Candidate {
            url: text(&value, "/html_url")?.into(),
            source_repository: text(&value, "/head/repo/full_name")?.into(),
            source_branch: text(&value, "/head/ref")?.into(),
            target_branch: text(&value, "/base/ref")?.into(),
            head: text(&value, "/head/sha")?.into(),
            base: text(&value, "/base/sha")?.into(),
            draft: value
                .get("draft")
                .and_then(Value::as_bool)
                .ok_or_else(|| Error("missing GitHub draft state".into()))?,
            open: text(&value, "/state")? == "open",
        })
    }

    fn checks(&mut self, candidate: &Candidate, policy: &Policy) -> Result<Vec<String>, Error> {
        let pr = PullRequest::parse(&candidate.url)?;
        let mut required = policy.required_checks.clone();
        let (status, protection) = self.request(
            "GET",
            &format!(
                "/repos/{}/branches/{}/protection",
                pr.repository,
                segment(&candidate.target_branch)
            ),
            None,
        )?;
        if status != 404 {
            if let Some(checks) = protection
                .pointer("/required_status_checks/checks")
                .and_then(Value::as_array)
            {
                for check in checks {
                    required.push(CheckRequirement {
                        name: text(check, "/context")?.into(),
                        app_id: check.get("app_id").and_then(Value::as_u64),
                    });
                }
            }
        }
        let mut checks = self.pages(
            &format!(
                "/repos/{}/commits/{}/check-runs?filter=latest",
                pr.repository, candidate.head
            ),
            Some("check_runs"),
        )?;
        let pull = self.get(&format!("/repos/{}/pulls/{}", pr.repository, pr.number))?;
        if let Some(merge_sha) = pull
            .get("merge_commit_sha")
            .and_then(Value::as_str)
            .filter(|s| *s != candidate.head)
        {
            let merge = self.get(&format!("/repos/{}/commits/{merge_sha}", pr.repository))?;
            let parents: BTreeSet<_> = merge
                .get("parents")
                .and_then(Value::as_array)
                .ok_or_else(|| Error("missing test-merge parents".into()))?
                .iter()
                .filter_map(|p| p.get("sha").and_then(Value::as_str))
                .collect();
            if parents == BTreeSet::from([candidate.head.as_str(), candidate.base.as_str()]) {
                checks.extend(self.pages(
                    &format!(
                        "/repos/{}/commits/{merge_sha}/check-runs?filter=latest",
                        pr.repository
                    ),
                    Some("check_runs"),
                )?);
            }
        }
        let mut latest: BTreeMap<(String, u64), Value> = BTreeMap::new();
        for check in checks {
            let key = (
                text(&check, "/name")?.to_owned(),
                number(&check, "/app/id")?,
            );
            if latest.get(&key).is_none_or(|old| {
                check.get("id").and_then(Value::as_u64) > old.get("id").and_then(Value::as_u64)
            }) {
                latest.insert(key, check);
            }
        }
        let statuses = self.pages(
            &format!(
                "/repos/{}/commits/{}/statuses",
                pr.repository, candidate.head
            ),
            None,
        )?;
        let mut legacy = BTreeMap::new();
        for value in statuses {
            legacy
                .entry(text(&value, "/context")?.to_owned())
                .or_insert(value);
        }
        let mut blockers = Vec::new();
        for check in &required {
            let successful = latest.iter().any(|((name, app), v)| {
                name == &check.name
                    && check.app_id.is_none_or(|expected| expected == *app)
                    && v.get("status").and_then(Value::as_str) == Some("completed")
                    && v.get("conclusion").and_then(Value::as_str) == Some("success")
            }) || (check.app_id.is_none()
                && legacy
                    .get(&check.name)
                    .is_some_and(|v| v.get("state").and_then(Value::as_str) == Some("success")));
            if !successful {
                blockers.push(format!(
                    "required CI context is missing or unsuccessful: {}",
                    check.name
                ));
            }
        }
        for ((name, _), v) in &latest {
            if v.get("status").and_then(Value::as_str) != Some("completed")
                || v.get("conclusion").and_then(Value::as_str) != Some("success")
            {
                blockers.push(format!("CI check has not passed: {name}"));
            }
        }
        for (name, v) in &legacy {
            if v.get("state").and_then(Value::as_str) != Some("success") {
                blockers.push(format!("CI status has not passed: {name}"));
            }
        }
        if required.is_empty() && latest.is_empty() && legacy.is_empty() {
            blockers.push("no CI qualification evidence or declared required contexts".into());
        }
        blockers.sort();
        blockers.dedup();
        Ok(blockers)
    }

    fn next_poll_at(&self) -> Option<u64> {
        self.retry_at.lock().ok().and_then(|v| *v)
    }
}

/// Greptile's documented GitHub trigger plus parent-review revision evidence.
pub struct GreptileGitHub {
    github: GitHub,
}

impl GreptileGitHub {
    /// Select the GitHub-comment transport explicitly.
    pub fn new(github: GitHub) -> Self {
        Self { github }
    }

    /// Record an evidence-backed false-positive disposition on the forge.
    /// Authorization is rechecked when the disposition is consumed.
    pub fn disposition(
        &self,
        candidate: &Candidate,
        policy: &Policy,
        review: &Review,
        finding: &Finding,
        reason: &str,
        evidence: &str,
    ) -> Result<(), Error> {
        if reason.trim().is_empty() || evidence.trim().is_empty() {
            return Err(Error(
                "dispositions require nonempty reason and evidence".into(),
            ));
        }
        let disposition = Disposition {
            head: candidate.head.clone(),
            base: candidate.base.clone(),
            policy_digest: policy.digest()?,
            review_id: review.id.clone(),
            finding_id: finding.id.clone(),
            body_digest: hex_digest(finding.body.as_bytes()),
            reason: reason.into(),
            evidence: evidence.into(),
        };
        let pr = PullRequest::parse(&candidate.url)?;
        let body = format!(
            "{DISPOSITION_MARKER}{} -->\n\nEvidence-backed finding disposition recorded by the review CLI.",
            serde_json::to_string(&disposition)?
        );
        self.github.request(
            "POST",
            &format!("/repos/{}/issues/{}/comments", pr.repository, pr.number),
            Some(json!({"body": body})),
        )?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Disposition {
    head: String,
    base: String,
    policy_digest: String,
    review_id: String,
    finding_id: String,
    body_digest: String,
    reason: String,
    evidence: String,
}

impl Provider for GreptileGitHub {
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
        candidate: &Candidate,
        policy: &Policy,
        local: Option<&Intent>,
    ) -> Result<Evidence, Error> {
        let pr = PullRequest::parse(&candidate.url)?;
        let reviews = self.github.pages(
            &format!("/repos/{}/pulls/{}/reviews", pr.repository, pr.number),
            None,
        )?;
        let known_review_ids = reviews
            .iter()
            .map(|r| number(r, "/id").map(|id| id.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let comments = self.github.pages(
            &format!("/repos/{}/issues/{}/comments", pr.repository, pr.number),
            None,
        )?;
        let mut permissions = BTreeMap::new();
        let mut recovered_intent = None;
        let mut receipt = None;
        let mut request_comment_id = 0;
        let mut requested_at = None;
        for comment in &comments {
            if let Some(intent) = marker::<Intent>(text(comment, "/body")?, REQUEST_MARKER) {
                if intent.matches(candidate, policy)
                    && local.is_none_or(|i| i.id == intent.id)
                    && self.github.authorized(&pr, comment, &mut permissions)?
                    && number(comment, "/id")? > request_comment_id
                {
                    request_comment_id = number(comment, "/id")?;
                    receipt = Some(text(comment, "/html_url")?.to_owned());
                    requested_at = Some(timestamp(
                        comment
                            .get("updated_at")
                            .and_then(Value::as_str)
                            .unwrap_or(text(comment, "/created_at")?),
                    )?);
                    recovered_intent = Some(intent);
                }
            }
        }
        let intent = local.or(recovered_intent.as_ref());
        let mut review = None;
        if let Some(intent) = intent.filter(|_| receipt.is_some()) {
            // Parent commit_id is authoritative even when GitHub remaps an
            // inline comment to a newer commit or marks its thread resolved.
            if let Some(parent) = reviews
                .iter()
                .rev()
                .find(|r| {
                    r.pointer("/user/id").and_then(Value::as_u64) == Some(policy.reviewer_id)
                        && r.pointer("/user/type").and_then(Value::as_str) == Some("Bot")
                        && r.get("commit_id").and_then(Value::as_str)
                            == Some(candidate.head.as_str())
                        && r.get("id")
                            .and_then(Value::as_u64)
                            .is_some_and(|id| !intent.baseline_review_ids.contains(&id.to_string()))
                })
                .filter(|r| {
                    // Do not fall back to an older passing review after a newer
                    // matching provider review is pending or dismissed.
                    matches!(
                        r.get("state").and_then(Value::as_str),
                        Some("COMMENTED" | "APPROVED" | "CHANGES_REQUESTED")
                    ) && r.get("submitted_at").and_then(Value::as_str).is_some()
                        && r.get("submitted_at")
                            .and_then(Value::as_str)
                            .and_then(|s| timestamp(s).ok())
                            .zip(requested_at)
                            .is_some_and(|(submitted, requested)| submitted >= requested)
                        && r.get("id")
                            .and_then(Value::as_u64)
                            .is_some_and(|id| !intent.baseline_review_ids.contains(&id.to_string()))
                })
            {
                let id = number(parent, "/id")?.to_string();
                let raw = self.github.pages(
                    &format!(
                        "/repos/{}/pulls/{}/reviews/{id}/comments",
                        pr.repository, pr.number
                    ),
                    None,
                )?;
                let mut findings = Vec::new();
                for item in raw {
                    if item.pointer("/user/id").and_then(Value::as_u64) != Some(policy.reviewer_id)
                    {
                        return Err(Error("review contains an unexpected finding author".into()));
                    }
                    let body = text(&item, "/body")?.to_owned();
                    findings.push(Finding {
                        id: number(&item, "/id")?.to_string(),
                        severity: severity(&body),
                        body,
                        path: item.get("path").and_then(Value::as_str).map(str::to_owned),
                        line: item
                            .get("line")
                            .and_then(Value::as_u64)
                            .or_else(|| item.get("original_line").and_then(Value::as_u64)),
                        provider_addressed: false,
                        url: text(&item, "/html_url")?.into(),
                    });
                }
                let body = text(parent, "/body")?;
                if !body.trim().is_empty() || text(parent, "/state")? == "CHANGES_REQUESTED" {
                    findings.push(Finding {
                        id: format!("review-{id}"),
                        body: body.into(),
                        severity: None,
                        path: None,
                        line: None,
                        provider_addressed: false,
                        url: text(parent, "/html_url")?.into(),
                    });
                }
                // Dispositions are exact-review, exact-body and exact-policy,
                // and can only be supplied by current repository writers.
                // Greptile may put additional feedback in issue comments. Such
                // comments do not attest a SHA, so retain them conservatively
                // as findings rather than claiming inline-only completeness.
                for comment in &comments {
                    if comment.pointer("/user/id").and_then(Value::as_u64)
                        == Some(policy.reviewer_id)
                        && timestamp(text(comment, "/created_at")?)?
                            >= requested_at.unwrap_or(i64::MAX)
                        && !text(comment, "/body")?.trim().is_empty()
                    {
                        findings.push(Finding {
                            id: format!("issue-{}", number(comment, "/id")?),
                            severity: severity(text(comment, "/body")?),
                            body: text(comment, "/body")?.into(),
                            path: None,
                            line: None,
                            provider_addressed: false,
                            url: text(comment, "/html_url")?.into(),
                        });
                    }
                }
                for comment in &comments {
                    if let Some(d) =
                        marker::<Disposition>(text(comment, "/body")?, DISPOSITION_MARKER)
                    {
                        if d.head == candidate.head
                            && d.base == candidate.base
                            && d.review_id == id
                            && d.policy_digest == policy.digest()?
                            && !d.reason.trim().is_empty()
                            && !d.evidence.trim().is_empty()
                            && self.github.authorized(&pr, comment, &mut permissions)?
                        {
                            findings.retain(|f| {
                                f.id != d.finding_id
                                    || hex_digest(f.body.as_bytes()) != d.body_digest
                            });
                        }
                    }
                }
                review = Some(Review {
                    id,
                    provider: policy.provider.clone(),
                    reviewed_head: Some(text(parent, "/commit_id")?.into()),
                    requested_base: Some(intent.candidate.base.clone()),
                    policy_digest: intent.policy_digest.clone(),
                    completed: true,
                    complete_findings: true,
                    findings,
                    url: text(parent, "/html_url")?.into(),
                });
            }
        }
        Ok(Evidence {
            known_review_ids,
            recovered_intent,
            request_receipt: receipt,
            review,
        })
    }

    fn submit(&mut self, intent: &Intent) -> Result<String, Error> {
        let pr = PullRequest::parse(&intent.candidate.url)?;
        let body = format!(
            "{REQUEST_MARKER}{} -->\n\n@greptileai {}\n\nRequested head: `{}`; observed target: `{}`. Review feedback is evidence for source triage.",
            serde_json::to_string(intent)?,
            if intent.candidate.draft {
                "review this draft"
            } else {
                "review this PR"
            },
            intent.candidate.head,
            intent.candidate.base
        );
        let (_, receipt) = self.github.request(
            "POST",
            &format!("/repos/{}/issues/{}/comments", pr.repository, pr.number),
            Some(json!({"body": body})),
        )?;
        Ok(text(&receipt, "/html_url")?.into())
    }

    fn next_poll_at(&self) -> Option<u64> {
        self.github.next_poll_at()
    }
}

fn text<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, Error> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| Error(format!("GitHub evidence missing required field {pointer}")))
}
fn number(value: &Value, pointer: &str) -> Result<u64, Error> {
    value
        .pointer(pointer)
        .and_then(Value::as_u64)
        .ok_or_else(|| Error(format!("GitHub evidence missing required field {pointer}")))
}
fn marker<T: serde::de::DeserializeOwned>(body: &str, prefix: &str) -> Option<T> {
    let json = body.strip_prefix(prefix)?.split_once(" -->")?.0;
    serde_json::from_str(json).ok()
}
fn segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}
fn same_comparison(a: &Candidate, b: &Candidate) -> bool {
    a.url == b.url
        && a.head == b.head
        && a.base == b.base
        && a.source_repository == b.source_repository
        && a.source_branch == b.source_branch
        && a.target_branch == b.target_branch
}
fn severity(body: &str) -> Option<String> {
    ["P0", "P1", "P2"]
        .into_iter()
        .find(|p| body.contains(&format!("alt=\"{p}\"")) || body.starts_with(&format!("**{p}")))
        .map(str::to_owned)
}
fn timestamp(value: &str) -> Result<i64, Error> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|t| t.timestamp())
        .map_err(|_| Error("invalid forge evidence timestamp".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn server(replies: Vec<(u16, Value)>) -> (GitHub, thread::JoinHandle<()>) {
        server_with_headers(replies, "")
    }

    fn server_with_headers(
        replies: Vec<(u16, Value)>,
        headers: &'static str,
    ) -> (GitHub, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for (status, value) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0; 4096];
                let _ = stream.read(&mut buffer).unwrap();
                let body = value.to_string();
                write!(stream, "HTTP/1.1 {status} Test\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let mut github = GitHub::new("fixture-credential".into()).unwrap();
        github.endpoint = format!("http://{address}");
        (github, handle)
    }

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
    fn intent() -> Intent {
        Intent {
            schema_version: 1,
            id: "fixture-request".into(),
            candidate: candidate(),
            policy_digest: Policy::default().digest().unwrap(),
            baseline_review_ids: vec![],
        }
    }
    fn marker_comment() -> Value {
        json!({"id":12, "body":format!("{REQUEST_MARKER}{} -->", serde_json::to_string(&intent()).unwrap()), "html_url":"https://github.com/example/project/pull/12#issuecomment-12", "created_at":"2026-10-03T12:00:00Z", "user":{"login":"writer"}})
    }
    fn parent(head: String, submitted: &str) -> Value {
        json!({"id":42, "commit_id":head, "state":"COMMENTED", "body":"", "submitted_at":submitted, "html_url":"https://github.com/example/project/pull/12#pullrequestreview-42", "user":{"id":165735046, "type":"Bot"}})
    }

    #[test]
    fn stale_parent_review_cannot_be_revived_by_inline_remapping() {
        let (github, handle) = server(vec![
            (200, json!([parent("c".repeat(40), "2026-10-03T12:01:00Z")])),
            (200, json!([marker_comment()])),
            (200, json!({"permission":"write"})),
        ]);
        let evidence = GreptileGitHub::new(github)
            .inspect(&candidate(), &Policy::default(), Some(&intent()))
            .unwrap();
        assert!(evidence.review.is_none());
        handle.join().unwrap();
    }

    #[test]
    fn review_older_than_dispatch_cannot_qualify_even_with_matching_head() {
        let (github, handle) = server(vec![
            (200, json!([parent("a".repeat(40), "2026-10-03T11:59:00Z")])),
            (200, json!([marker_comment()])),
            (200, json!({"permission":"write"})),
        ]);
        assert!(
            GreptileGitHub::new(github)
                .inspect(&candidate(), &Policy::default(), None)
                .unwrap()
                .review
                .is_none()
        );
        handle.join().unwrap();
    }

    #[test]
    fn read_only_contributor_cannot_supply_acceptance_markers() {
        let (github, handle) = server(vec![
            (200, json!([parent("a".repeat(40), "2026-10-03T12:01:00Z")])),
            (200, json!([marker_comment()])),
            (404, Value::Null),
        ]);
        let evidence = GreptileGitHub::new(github)
            .inspect(&candidate(), &Policy::default(), None)
            .unwrap();
        assert!(evidence.recovered_intent.is_none());
        assert!(evidence.review.is_none());
        handle.join().unwrap();
    }

    #[test]
    fn editing_an_old_request_marker_cannot_adopt_a_preexisting_review() {
        let mut marker = marker_comment();
        marker["updated_at"] = json!("2026-10-03T12:02:00Z");
        let (github, handle) = server(vec![
            (200, json!([parent("a".repeat(40), "2026-10-03T12:01:00Z")])),
            (200, json!([marker])),
            (200, json!({"permission":"write"})),
        ]);
        assert!(
            GreptileGitHub::new(github)
                .inspect(&candidate(), &Policy::default(), None)
                .unwrap()
                .review
                .is_none()
        );
        handle.join().unwrap();
    }

    #[test]
    fn provider_issue_comment_feedback_is_not_silently_omitted() {
        let summary = json!({"id":19,"body":"Unresolved race found in the review summary", "created_at":"2026-10-03T12:01:00Z", "user":{"id":165735046}, "html_url":"https://github.com/example/project/pull/12#issuecomment-19"});
        let (github, handle) = server(vec![
            (200, json!([parent("a".repeat(40), "2026-10-03T12:01:00Z")])),
            (200, json!([marker_comment(), summary])),
            (200, json!({"permission":"write"})),
            (200, json!([])),
        ]);
        let review = GreptileGitHub::new(github)
            .inspect(&candidate(), &Policy::default(), None)
            .unwrap()
            .review
            .unwrap();
        assert!(review.findings.iter().any(|f| f.id == "issue-19"));
        assert_eq!(
            Policy::default().evaluate(&candidate(), &review),
            super::super::Verdict::Findings
        );
        handle.join().unwrap();
    }

    #[test]
    fn unknown_severity_and_remapped_inline_commit_remain_findings() {
        let (github, handle) = server(vec![
            (200, json!([parent("a".repeat(40), "2026-10-03T12:01:00Z")])),
            (200, json!([marker_comment()])),
            (200, json!({"permission":"write"})),
            (
                200,
                json!([{"id":17,"body":"Check this race","path":"src/lib.rs","line":null,"original_line":12,"commit_id":"c".repeat(40),"html_url":"https://github.com/example/project/pull/12#discussion_r17","user":{"id":165735046}}]),
            ),
        ]);
        let review = GreptileGitHub::new(github)
            .inspect(&candidate(), &Policy::default(), None)
            .unwrap()
            .review
            .unwrap();
        assert_eq!(review.reviewed_head, Some("a".repeat(40)));
        assert_eq!(review.findings[0].line, Some(12));
        assert_eq!(review.findings[0].severity, None);
        assert_eq!(
            Policy::default().evaluate(&candidate(), &review),
            super::super::Verdict::Findings
        );
        handle.join().unwrap();
    }

    #[test]
    fn all_pages_are_required_and_total_mismatch_is_rejected() {
        let hundred = vec![json!({"id":1}); 100];
        let (github, handle) = server(vec![(200, json!(hundred)), (200, json!([{"id":2}]))]);
        assert_eq!(github.pages("/fixture", None).unwrap().len(), 101);
        handle.join().unwrap();
        let (github, handle) = server(vec![(200, json!({"check_runs":[],"total_count":2}))]);
        assert!(github.pages("/fixture", Some("check_runs")).is_err());
        handle.join().unwrap();
    }

    #[test]
    fn transport_errors_do_not_print_response_secrets() {
        let (github, handle) = server(vec![(401, json!({"message":"fixture-credential"}))]);
        let error = github.get("/fixture").unwrap_err().to_string();
        assert!(!error.contains("fixture-credential"));
        assert!(error.contains("401"));
        handle.join().unwrap();
    }

    #[test]
    fn a_newer_dismissed_review_cannot_revive_an_older_passing_review() {
        let mut dismissed = parent("a".repeat(40), "2026-10-03T12:02:00Z");
        dismissed["id"] = json!(43);
        dismissed["state"] = json!("DISMISSED");
        let (github, handle) = server(vec![
            (
                200,
                json!([parent("a".repeat(40), "2026-10-03T12:01:00Z"), dismissed]),
            ),
            (200, json!([marker_comment()])),
            (200, json!({"permission":"write"})),
        ]);
        assert!(
            GreptileGitHub::new(github)
                .inspect(&candidate(), &Policy::default(), None)
                .unwrap()
                .review
                .is_none()
        );
        handle.join().unwrap();
    }

    #[test]
    fn missing_ci_is_a_blocker_even_after_a_completed_review() {
        let (mut github, handle) = server(vec![
            (404, Value::Null),
            (200, json!({"check_runs":[],"total_count":0})),
            (200, json!({"merge_commit_sha":null})),
            (200, json!([])),
        ]);
        assert!(
            !github
                .checks(&candidate(), &Policy::default())
                .unwrap()
                .is_empty()
        );
        handle.join().unwrap();
    }

    #[test]
    fn rate_limit_retain_retry_after_without_disclosing_the_response() {
        let (github, handle) = server_with_headers(
            vec![(429, json!({"message":"fixture-credential"}))],
            "Retry-After: 120\r\n",
        );
        let before = now_seconds();
        let error = github.get("/fixture").unwrap_err();
        assert!(github.next_poll_at().unwrap() >= before + 120);
        assert!(!error.to_string().contains("fixture-credential"));
        handle.join().unwrap();
    }
}
