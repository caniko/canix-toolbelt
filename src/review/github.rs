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

pub(super) mod roborev;
pub(super) mod roborev_producer;

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
        self.merge_with_policy(candidate, Some(policy), true)
    }

    /// Validate CI and native protection without loading or contacting a reviewer.
    /// `apply` performs an explicitly authorized merge; otherwise this is read-only.
    pub fn merge_native(&mut self, candidate: &Candidate, apply: bool) -> Result<Value, Error> {
        self.merge_with_policy(candidate, None, apply)
    }

    fn merge_with_policy(
        &mut self,
        candidate: &Candidate,
        policy: Option<&Policy>,
        apply: bool,
    ) -> Result<Value, Error> {
        candidate.validate()?;
        if let Some(policy) = policy {
            policy.validate()?;
        }
        let current = self.candidate(&candidate.url)?;
        if !same_comparison(candidate, &current) || !current.open || current.draft {
            return Err(Error(
                "merge candidate moved, closed or remains draft; refresh the candidate".into(),
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
        if let Some(policy) = policy {
            let checks = required
                .get("checks")
                .and_then(Value::as_array)
                .ok_or_else(|| Error("protected review-policy check is not configured".into()))?;
            let policy_app = policy.policy_app_id()?;
            if !checks.iter().any(|c| {
                c.get("context").and_then(Value::as_str) == Some("review-policy")
                    && c.get("app_id").and_then(Value::as_u64) == Some(policy_app)
            }) {
                return Err(Error(
                    "merge requires review-policy bound to the configured dedicated policy GitHub App"
                        .into(),
                ));
            }
        }
        let blockers = self.checks_required(
            candidate,
            policy.map_or(&[], |policy| policy.required_checks.as_slice()),
            policy.is_some(),
        )?;
        if !blockers.is_empty() {
            return Err(Error(format!("merge CI gate: {}", blockers.join("; "))));
        }
        let fresh = self.candidate(&candidate.url)?;
        if !same_comparison(candidate, &fresh) {
            return Err(Error("comparison changed immediately before merge".into()));
        }
        // Explicit review-gated callers still revalidate live provider evidence.
        if let Some(policy) = policy {
            let evidence =
                super::github_provider(self.clone(), policy)?.inspect(&fresh, policy, None)?;
            if evidence
                .review
                .as_ref()
                .is_none_or(|review| policy.evaluate(&fresh, review) != super::Verdict::Ready)
            {
                return Err(Error(
                    "completed current provider review no longer qualifies; rerun ensure".into(),
                ));
            }
        }
        let final_candidate = self.candidate(&candidate.url)?;
        if !same_comparison(candidate, &final_candidate)
            || !final_candidate.open
            || final_candidate.draft
        {
            return Err(Error(
                "comparison changed during final merge validation".into(),
            ));
        }
        if !apply {
            return Ok(
                json!({"verdict":"ready", "candidate":candidate, "applied":false, "reviewRequired":policy.is_some()}),
            );
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
        policy: &Policy,
        ready: bool,
        details_url: &str,
    ) -> Result<(), Error> {
        let pr = PullRequest::parse(&candidate.url)?;
        candidate.validate()?;
        let app = policy.policy_app_id()?;
        let (_, check) = self.request("POST", &format!("/repos/{}/check-runs", pr.repository), Some(json!({
            "name": "review-policy", "head_sha": candidate.head, "status": "completed",
            "conclusion": if ready { "success" } else { "failure" },
            "details_url": details_url,
            "output": {"title": if ready { "Current review qualifies" } else { "Review evidence does not qualify" },
                "summary": "Revision-bound review policy evaluated by canix-toolbelt. Required CI remains independently enforced."}
        })))?;
        if check.pointer("/app/id").and_then(Value::as_u64) != Some(app) {
            return Err(Error(
                "policy check was not published by the configured dedicated GitHub App".into(),
            ));
        }
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
        self.checks_required(candidate, &policy.required_checks, true)
    }

    fn next_poll_at(&self) -> Option<u64> {
        self.retry_at.lock().ok().and_then(|v| *v)
    }
}

impl GitHub {
    fn checks_required(
        &mut self,
        candidate: &Candidate,
        additional: &[CheckRequirement],
        all_ci: bool,
    ) -> Result<Vec<String>, Error> {
        let pr = PullRequest::parse(&candidate.url)?;
        let mut required = additional.to_vec();
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
            if let Some(contexts) = protection
                .pointer("/required_status_checks/contexts")
                .and_then(Value::as_array)
            {
                for context in contexts {
                    let name = context
                        .as_str()
                        .ok_or_else(|| Error("invalid native required status context".into()))?;
                    if !required.iter().any(|r| r.name == name) {
                        required.push(CheckRequirement {
                            name: name.into(),
                            app_id: None,
                        });
                    }
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
        let mut qualified_shas = vec![candidate.head.clone()];
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
                qualified_shas.push(merge_sha.into());
                checks.extend(self.pages(
                    &format!(
                        "/repos/{}/commits/{merge_sha}/check-runs?filter=latest",
                        pr.repository
                    ),
                    Some("check_runs"),
                )?);
            }
        }
        let mut latest: BTreeMap<(String, u64, String), Value> = BTreeMap::new();
        for check in checks {
            let sha = text(&check, "/head_sha")?;
            if !qualified_shas.iter().any(|expected| expected == sha) {
                return Err(Error("CI check belongs to a different comparison".into()));
            }
            let key = (
                text(&check, "/name")?.to_owned(),
                number(&check, "/app/id")?,
                sha.into(),
            );
            if latest.get(&key).is_none_or(|old| {
                check.get("id").and_then(Value::as_u64) > old.get("id").and_then(Value::as_u64)
            }) {
                latest.insert(key, check);
            }
        }
        let mut legacy = BTreeMap::new();
        for sha in qualified_shas {
            for value in self.pages(
                &format!("/repos/{}/commits/{sha}/statuses", pr.repository),
                None,
            )? {
                legacy
                    .entry((text(&value, "/context")?.to_owned(), sha.clone()))
                    .or_insert(value);
            }
        }
        let mut blockers = Vec::new();
        let optional_review = |name: &str| {
            name == "review-policy" && !required.iter().any(|check| check.name == name)
        };
        // Native merges enforce declared requirements. Advisory provider jobs
        // are not promoted to requirements merely because they publish checks.
        // With no declared CI, retain the full qualification-evidence fallback.
        // The review-only policy App cannot supply CI qualification itself.
        let qualification_fallback = required.iter().all(|check| check.name == "review-policy");
        let inspect_present = |name: &str, app: Option<u64>| {
            !optional_review(name)
                && (all_ci
                    || qualification_fallback
                    || required.iter().any(|check| {
                        check.name == name
                            && check.app_id.is_none_or(|expected| app == Some(expected))
                    }))
        };
        for check in &required {
            let successful = latest.iter().any(|((name, app, _), v)| {
                name == &check.name
                    && check.app_id.is_none_or(|expected| expected == *app)
                    && v.get("status").and_then(Value::as_str) == Some("completed")
                    && v.get("conclusion").and_then(Value::as_str) == Some("success")
            }) || (check.app_id.is_none()
                && legacy.iter().any(|((name, _), v)| {
                    name == &check.name && v.get("state").and_then(Value::as_str) == Some("success")
                }));
            if !successful {
                blockers.push(format!(
                    "required CI context is missing or unsuccessful: {}",
                    check.name
                ));
            }
        }
        for ((name, app, _), v) in &latest {
            if !inspect_present(name, Some(*app)) {
                continue;
            }
            if v.get("status").and_then(Value::as_str) != Some("completed")
                || v.get("conclusion").and_then(Value::as_str) != Some("success")
            {
                blockers.push(format!("CI check has not passed: {name}"));
            }
        }
        for ((name, _), v) in &legacy {
            if !inspect_present(name, None) {
                continue;
            }
            if v.get("state").and_then(Value::as_str) != Some("success") {
                blockers.push(format!("CI status has not passed: {name}"));
            }
        }
        if qualification_fallback
            && latest.keys().all(|(name, _, _)| name == "review-policy")
            && legacy.keys().all(|(name, _)| name == "review-policy")
        {
            blockers.push("no CI qualification evidence or declared required contexts".into());
        }
        blockers.sort();
        blockers.dedup();
        Ok(blockers)
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
        self.github
            .disposition(candidate, policy, review, finding, reason, evidence)
    }
}

impl GitHub {
    /// Record a provider-neutral exact-finding disposition. Collection must
    /// recheck the writer's authorization and every bound identity.
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
        self.request(
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
        if policy.provider != "greptile" || policy.transport != "github-comment" {
            return Err(Error(
                "Greptile adapter requires an explicitly selected Greptile policy".into(),
            ));
        }
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
                let body = text(parent, "/body")?;
                if credit_limit_notice(body) {
                    return Err(Error(
                        "Greptile review unavailable: account credit limit reached; restore provider capacity and request a fresh review".into(),
                    ));
                }
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
                        correlated: true,
                        url: text(&item, "/html_url")?.into(),
                    });
                }
                if !body.trim().is_empty() || text(parent, "/state")? == "CHANGES_REQUESTED" {
                    findings.push(Finding {
                        id: format!("review-{id}"),
                        body: body.into(),
                        severity: None,
                        path: None,
                        line: None,
                        provider_addressed: false,
                        correlated: true,
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
                        && timestamp(
                            comment
                                .get("updated_at")
                                .and_then(Value::as_str)
                                .unwrap_or(text(comment, "/created_at")?),
                        )? >= requested_at.unwrap_or(i64::MAX)
                        && !text(comment, "/body")?.trim().is_empty()
                    {
                        findings.push(Finding {
                            id: format!("issue-{}", number(comment, "/id")?),
                            severity: severity(text(comment, "/body")?),
                            body: text(comment, "/body")?.into(),
                            path: None,
                            line: None,
                            provider_addressed: false,
                            correlated: text(comment, "/body")?
                                .contains(text(parent, "/html_url")?)
                                || findings.iter().any(|f| {
                                    f.correlated
                                        && text(comment, "/body")
                                            .is_ok_and(|body| body.contains(&f.url))
                                }),
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
                    provider_job_id: None,
                    provider_review_id: None,
                    provider: policy.provider.clone(),
                    reviewed_head: Some(text(parent, "/commit_id")?.into()),
                    requested_base: Some(intent.candidate.base.clone()),
                    reviewed_base: None,
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
fn credit_limit_notice(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    body.contains("has reached")
        && body.contains("credit limit")
        && body.contains("to continue receiving code reviews")
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

    pub(super) fn server(replies: Vec<(u16, Value)>) -> (GitHub, thread::JoinHandle<()>) {
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
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                let header_end = loop {
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0, "incomplete fixture HTTP request");
                    request.extend_from_slice(&buffer[..count]);
                    assert!(request.len() <= 64 * 1024, "oversized fixture request");
                    if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let request_headers = std::str::from_utf8(&request[..header_end]).unwrap();
                let body_size = request_headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                assert!(
                    header_end + body_size <= 64 * 1024,
                    "oversized fixture request body"
                );
                while request.len() < header_end + body_size {
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0, "incomplete fixture request body");
                    request.extend_from_slice(&buffer[..count]);
                }
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

    fn protected_policy() -> Policy {
        Policy {
            required_checks: vec![CheckRequirement {
                name: "review-policy".into(),
                app_id: Some(456),
            }],
            ..Policy::default()
        }
    }

    #[test]
    fn unavailable_optional_review_does_not_block_passing_ci() {
        let current = candidate();
        let (mut github, handle) = server(vec![
            (
                200,
                json!({"required_status_checks":{"checks":[{"context":"CI","app_id":15368}]}}),
            ),
            (
                200,
                json!({"check_runs":[
                    {"id":1,"name":"CI","head_sha":current.head,"app":{"id":15368},"status":"completed","conclusion":"success"},
                    {"id":2,"name":"review-policy","head_sha":current.head,"app":{"id":456},"status":"completed","conclusion":"failure"}
                ]}),
            ),
            (200, json!({"merge_commit_sha":null})),
            (200, json!([])),
        ]);
        let blockers = github.checks(&current, &Policy::default()).unwrap();
        handle.join().unwrap();
        assert!(blockers.is_empty(), "{blockers:?}");
    }

    fn native_merge_replies(ci: &str, required_review: bool) -> Vec<(u16, Value)> {
        let current = candidate();
        let pull = json!({"html_url":current.url,"head":{"sha":current.head,"ref":current.source_branch,"repo":{"full_name":current.source_repository}},
            "base":{"sha":current.base,"ref":current.target_branch},"state":"open","draft":false,"merge_commit_sha":null});
        let mut checks = vec![json!({"context":"CI","app_id":15368})];
        if required_review {
            checks.push(json!({"context":"review-policy","app_id":456}));
        }
        let protection = json!({"enforce_admins":{"enabled":true},"required_status_checks":{"strict":true,"checks":checks}});
        vec![
            (200, pull.clone()),
            (200, protection.clone()),
            (200, protection),
            (
                200,
                json!({"check_runs":[
                {"id":1,"name":"CI","head_sha":current.head,"app":{"id":15368},"status":"completed","conclusion":ci},
                {"id":2,"name":"review-policy","head_sha":current.head,"app":{"id":456},"status":"completed","conclusion":"failure"},
                {"id":3,"name":"advisory-review-coordinator","head_sha":current.head,"app":{"id":15368},"status":"completed","conclusion":"failure"}
                ]}),
            ),
            (200, pull.clone()),
            (200, json!([])),
            (200, pull.clone()),
            (200, pull),
        ]
    }

    #[test]
    fn native_merge_and_preview_work_without_provider_evidence() {
        for apply in [false, true] {
            let mut replies = native_merge_replies("success", false);
            if apply {
                replies.push((200, json!({"merged":true})));
            }
            let (mut github, handle) = server(replies);
            let outcome = github.merge_native(&candidate(), apply).unwrap();
            handle.join().unwrap();
            if apply {
                assert_eq!(outcome["merged"], true);
            } else {
                assert_eq!(outcome["verdict"], "ready");
                assert_eq!(outcome["applied"], false);
                assert_eq!(outcome["reviewRequired"], false);
            }
        }
    }

    #[test]
    fn native_merge_preserves_failed_ci_and_required_review_blockers() {
        for (ci, required_review) in [
            ("failure", false),
            ("skipped", false),
            ("cancelled", false),
            ("success", true),
        ] {
            let mut replies = native_merge_replies(ci, required_review);
            replies.truncate(6);
            let (mut github, handle) = server(replies);
            let error = github.merge_native(&candidate(), true).unwrap_err();
            handle.join().unwrap();
            assert!(error.to_string().contains(if required_review {
                "review-policy"
            } else {
                "CI"
            }));
        }
    }

    #[test]
    fn native_merge_rejects_a_moving_comparison_and_forge_refusal() {
        let mut replies = native_merge_replies("success", false);
        replies.last_mut().unwrap().1["base"]["sha"] = json!("c".repeat(40));
        let (mut github, handle) = server(replies);
        assert!(
            github
                .merge_native(&candidate(), true)
                .unwrap_err()
                .to_string()
                .contains("comparison changed")
        );
        handle.join().unwrap();

        let mut replies = native_merge_replies("success", false);
        replies.push((405, json!({"message":"Required human approval is missing"})));
        let (mut github, handle) = server(replies);
        assert!(github.merge_native(&candidate(), true).is_err());
        handle.join().unwrap();
    }

    #[test]
    fn advisory_review_alone_cannot_qualify_ci() {
        let (mut github, handle) = server(vec![
            (404, Value::Null),
            (
                200,
                json!({"check_runs":[{"id":1,"name":"review-policy","head_sha":"a".repeat(40),"app":{"id":456},"status":"completed","conclusion":"success"}]}),
            ),
            (200, json!({"merge_commit_sha":null})),
            (200, json!([])),
        ]);
        let blockers = github.checks(&candidate(), &Policy::default()).unwrap();
        handle.join().unwrap();
        assert!(
            blockers
                .iter()
                .any(|blocker| blocker.contains("no CI qualification"))
        );
    }

    fn native_ci_fixture(
        required: Vec<Value>,
        checks: Vec<Value>,
        statuses: Vec<Value>,
    ) -> (GitHub, thread::JoinHandle<()>) {
        server(vec![
            (200, json!({"required_status_checks":{"checks":required}})),
            (200, json!({"check_runs":checks})),
            (200, json!({"merge_commit_sha":null})),
            (200, json!(statuses)),
        ])
    }

    fn ci_run(id: u64, name: &str, app: u64, conclusion: &str) -> Value {
        json!({"id":id,"name":name,"head_sha":candidate().head,"app":{"id":app},
            "status":if conclusion == "pending" {"in_progress"} else {"completed"},
            "conclusion":if conclusion == "pending" {Value::Null} else {json!(conclusion)}})
    }

    #[test]
    fn review_only_protection_keeps_ci_qualification_fallback() {
        for conclusion in [
            Some("failure"),
            Some("pending"),
            Some("cancelled"),
            Some("skipped"),
            Some("success"),
            None,
        ] {
            let mut checks = vec![ci_run(1, "review-policy", 456, "success")];
            if let Some(conclusion) = conclusion {
                checks.push(ci_run(2, "CI", 15368, conclusion));
            }
            let (mut github, handle) = native_ci_fixture(
                vec![json!({"context":"review-policy","app_id":456})],
                checks,
                vec![],
            );
            let blockers = github.checks_required(&candidate(), &[], false).unwrap();
            handle.join().unwrap();
            if conclusion == Some("success") {
                assert!(blockers.is_empty(), "{blockers:?}");
            } else if conclusion.is_none() {
                assert!(
                    blockers.iter().any(|b| b.contains("no CI qualification")),
                    "{blockers:?}"
                );
            } else {
                assert!(
                    blockers
                        .iter()
                        .any(|b| b.contains("CI check has not passed: CI")),
                    "{blockers:?}"
                );
            }
        }
        for state in ["pending", "error", "failure", "success"] {
            let (mut github, handle) = native_ci_fixture(
                vec![json!({"context":"review-policy","app_id":456})],
                vec![ci_run(1, "review-policy", 456, "success")],
                vec![json!({"context":"CI","state":state})],
            );
            let blockers = github.checks_required(&candidate(), &[], false).unwrap();
            handle.join().unwrap();
            assert_eq!(blockers.is_empty(), state == "success", "{blockers:?}");
        }
    }

    #[test]
    fn native_ci_filter_keeps_required_app_identity_for_advisory_names() {
        for all_ci in [false, true] {
            for legacy in [false, true] {
                let mut checks = vec![ci_run(1, "CI", 15368, "success")];
                let mut statuses = vec![];
                if legacy {
                    statuses.push(json!({"context":"CI","state":"failure"}));
                } else {
                    checks.push(ci_run(2, "CI", 789, "failure"));
                }
                let (mut github, handle) = native_ci_fixture(
                    vec![json!({"context":"CI","app_id":15368})],
                    checks,
                    statuses,
                );
                let blockers = github.checks_required(&candidate(), &[], all_ci).unwrap();
                handle.join().unwrap();
                assert_eq!(blockers.is_empty(), !all_ci, "{blockers:?}");
            }
        }
        for app in [Some(15368), None] {
            let (mut github, handle) = native_ci_fixture(
                vec![json!({"context":"CI","app_id":app})],
                vec![
                    ci_run(1, "CI", 15368, "failure"),
                    ci_run(2, "CI", 789, "success"),
                ],
                vec![json!({"context":"CI","state":"success"})],
            );
            let blockers = github.checks_required(&candidate(), &[], false).unwrap();
            handle.join().unwrap();
            assert!(
                blockers
                    .iter()
                    .any(|b| b.contains("CI check has not passed: CI")),
                "{blockers:?}"
            );
            if app.is_some() {
                assert!(
                    blockers
                        .iter()
                        .any(|b| b.contains("required CI context is missing")),
                    "{blockers:?}"
                );
            }
        }
    }

    #[test]
    fn native_required_review_cannot_be_replaced_by_another_app_or_legacy_status() {
        for app in [456, 789] {
            let (mut github, handle) = native_ci_fixture(
                vec![
                    json!({"context":"review-policy","app_id":456}),
                    json!({"context":"CI","app_id":15368}),
                ],
                vec![
                    ci_run(1, "review-policy", app, "failure"),
                    ci_run(2, "review-policy", 789, "success"),
                    ci_run(3, "CI", 15368, "success"),
                ],
                vec![json!({"context":"review-policy","state":"success"})],
            );
            let blockers = github.checks_required(&candidate(), &[], false).unwrap();
            handle.join().unwrap();
            assert!(
                blockers
                    .iter()
                    .any(|b| b
                        .contains("required CI context is missing or unsuccessful: review-policy")),
                "{blockers:?}"
            );
        }
    }

    #[test]
    fn merge_rejects_a_different_dedicated_app_even_with_strict_protection() {
        let current = candidate();
        let (mut github, handle) = server(vec![
            (
                200,
                json!({"html_url":current.url,"head":{"sha":current.head,"ref":current.source_branch,"repo":{"full_name":current.source_repository}},
                "base":{"sha":current.base,"ref":current.target_branch},"state":"open","draft":false}),
            ),
            (
                200,
                json!({"enforce_admins":{"enabled":true},"required_status_checks":{"strict":true,"checks":[{"context":"review-policy","app_id":789}]}}),
            ),
        ]);
        let error = github.merge(&current, &protected_policy()).unwrap_err();
        assert!(error.to_string().contains("configured dedicated"));
        handle.join().unwrap();
    }

    #[test]
    fn policy_check_publication_requires_exact_app_response() {
        for identity in [json!(456), json!(789), json!(15368), Value::Null] {
            let (github, handle) = server(vec![(201, json!({"app":{"id":identity}}))]);
            let published = github.publish_check(
                &candidate(),
                &protected_policy(),
                true,
                "https://github.com/example/project/actions/runs/1",
            );
            assert_eq!(
                published.is_ok(),
                identity == json!(456),
                "identity {identity}: {published:?}"
            );
            handle.join().unwrap();
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
        let summary = json!({"id":19,"body":"Unresolved race found in the review summary", "created_at":"2026-10-03T11:59:00Z", "updated_at":"2026-10-03T12:01:00Z", "user":{"id":165735046}, "html_url":"https://github.com/example/project/pull/12#issuecomment-19"});
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
        assert!(
            !review
                .findings
                .iter()
                .find(|f| f.id == "issue-19")
                .unwrap()
                .correlated
        );
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
    fn missing_native_legacy_context_blocks_an_otherwise_passing_check() {
        let (mut github, handle) = server(vec![
            (
                200,
                json!({"required_status_checks":{"contexts":["required-legacy"],"checks":[]}}),
            ),
            (
                200,
                json!({"check_runs":[{"id":2,"name":"tests","app":{"id":15368},"head_sha":"a".repeat(40),"status":"completed","conclusion":"success"}],"total_count":1}),
            ),
            (200, json!({"merge_commit_sha":null})),
            (200, json!([])),
        ]);
        assert!(
            github
                .checks(&candidate(), &Policy::default())
                .unwrap()
                .iter()
                .any(|b| b.contains("required-legacy"))
        );
        handle.join().unwrap();
    }

    #[test]
    fn a_newer_passing_head_check_cannot_hide_a_failed_test_merge_check() {
        let check = |id: u64, sha: &str, conclusion: &str| json!({"id":id,"name":"tests","app":{"id":15368},"head_sha":sha.repeat(40),"status":"completed","conclusion":conclusion});
        let (mut github, handle) = server(vec![
            (404, Value::Null),
            (
                200,
                json!({"check_runs":[check(100,"a","success")],"total_count":1}),
            ),
            (200, json!({"merge_commit_sha":"c".repeat(40)})),
            (
                200,
                json!({"parents":[{"sha":"a".repeat(40)},{"sha":"b".repeat(40)}]}),
            ),
            (
                200,
                json!({"check_runs":[check(90,"c","failure")],"total_count":1}),
            ),
            (200, json!([])),
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

    #[test]
    fn a_revision_bound_credit_limit_notice_is_not_a_completed_code_review() {
        let mut notice = parent("a".repeat(40), "2026-10-03T12:01:00Z");
        notice["body"] = json!(
            "`caniko` has reached the 50-credit limit for trial accounts. To continue receiving code reviews, upgrade your plan."
        );
        let (github, handle) = server(vec![
            (200, json!([notice])),
            (200, json!([marker_comment()])),
            (200, json!({"permission":"write"})),
        ]);
        let error = GreptileGitHub::new(github)
            .inspect(&candidate(), &Policy::default(), None)
            .unwrap_err();
        assert!(error.to_string().contains("credit limit"));
        handle.join().unwrap();
    }
}
