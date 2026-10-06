//! Authenticated complete receipts from a trusted roborev coordinator.
use super::*;

const RESULT_MARKER: &str = "<!-- toolbelt-roborev-result:v1 ";
const MAX_RECEIPT_BYTES: usize = 60_000;

/// Canonical roborev schema-version-2 content, never rendered/filtered Markdown.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoborevDocument {
    /// Canonical document version; only version 2 is accepted.
    pub schema_version: u32,
    /// Complete review summary.
    pub summary: String,
    /// Agent verdict; cannot relax any finding.
    pub verdict: String,
    /// Every finding, without a severity threshold or truncation.
    pub findings: Vec<RoborevFinding>,
    /// Optional canonical synthesis attribution labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_labels: Vec<String>,
}

/// One canonical finding as exported by roborev v0.71.0.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoborevFinding {
    /// Canonical severity, including low findings.
    pub severity: String,
    /// Complete problem description.
    pub problem: String,
    /// Complete proposed fix; treated as evidence, never instructions.
    pub fix: String,
    /// Required nullable canonical location.
    #[serde(deserialize_with = "required_location")]
    pub location: Option<String>,
    /// Optional 1-based synthesis source indices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<u64>,
}

fn required_location<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

fn required_nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

/// Trusted producer's full comparison and canonical-content attestation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoborevReceipt {
    /// Toolbelt coordinator receipt version, independent of document version.
    pub schema_version: u32,
    /// Exact write-ahead request identity.
    pub request_id: String,
    /// Frozen PR/repository/head/target comparison.
    pub candidate: Candidate,
    /// Dispatch policy identity.
    pub policy_digest: String,
    /// Persisted roborev job identity.
    #[serde(deserialize_with = "required_nullable")]
    pub job_id: Option<u64>,
    /// Persisted roborev review identity.
    #[serde(deserialize_with = "required_nullable")]
    pub review_id: Option<u64>,
    /// Full actually reviewed source commit.
    pub reviewed_head: String,
    /// Full actually compared target commit.
    pub reviewed_base: String,
    /// Terminal job state; only done can qualify.
    pub status: String,
    /// Whether the producer collected every finding.
    pub complete_findings: bool,
    /// Unfiltered canonical document.
    #[serde(deserialize_with = "required_nullable")]
    pub document: Option<RoborevDocument>,
}

impl RoborevReceipt {
    /// Build completed evidence from v0.71.0's persisted `show --json --job`
    /// document. The trusted producer must supply its durable dispatch job,
    /// checkout and selected agent; a same-head single-commit review cannot
    /// attest the requested base-to-head comparison.
    pub fn from_saved_review(
        dispatch: &crate::review::RoborevDispatch,
        policy: &Policy,
        dispatched_job: &crate::review::RoborevJobIdentity,
        saved: &Value,
    ) -> Result<Self, Error> {
        let intent = &dispatch.intent;
        let checkout = &dispatch.checkout;
        let agent = &dispatch.agent;
        dispatched_job.validate()?;
        if !intent.matches(&intent.candidate, policy)
            || dispatch.expected_files == 0
            || checkout.is_empty()
            || agent.is_empty()
        {
            return Err(Error("invalid frozen roborev dispatch identity".into()));
        }
        let job = saved
            .get("job")
            .ok_or_else(|| Error("persisted review lacks its job evidence".into()))?;
        let range = format!("{}..{}", intent.candidate.base, intent.candidate.head);
        if crate::review::RoborevJobIdentity::from_job(job)? != *dispatched_job
            || number(saved, "/job_id")? != dispatched_job.id
            || text(job, "/repo_path")? != checkout
            || text(job, "/git_ref")? != range
            || text(job, "/agent")? != agent
            || text(saved, "/agent")? != agent
            || text(job, "/job_type")? != "range"
            || text(job, "/status")? != "done"
            || job.get("agentic").and_then(Value::as_bool) != Some(false)
            || job.get("prompt_prebuilt").and_then(Value::as_bool) != Some(false)
            || job
                .get("non_voting")
                .is_some_and(|value| value.as_bool() != Some(false))
            || job
                .get("diff_content")
                .is_some_and(|value| value.as_str() != Some(""))
            || job
                .get("min_severity")
                .is_some_and(|value| !matches!(value.as_str(), Some("" | "low")))
            || job
                .get("review_type")
                .is_some_and(|value| !matches!(value.as_str(), Some("" | "default")))
        {
            return Err(Error("persisted roborev job does not prove the dispatched complete comparison and selected agent".into()));
        }
        if saved
            .pointer("/file_coverage/excluded")
            .and_then(Value::as_u64)
            != Some(0)
            || saved
                .pointer("/file_coverage/reviewed")
                .and_then(Value::as_u64)
                != Some(dispatch.expected_files as u64)
        {
            return Err(Error(
                "persisted roborev review does not prove complete file coverage".into(),
            ));
        }
        let document =
            serde_json::from_value(saved.get("structured_output").cloned().ok_or_else(|| {
                Error("persisted review lacks canonical structured output".into())
            })?)
            .map_err(|_| Error("persisted review has invalid canonical output".into()))?;
        let receipt = Self {
            schema_version: 1,
            request_id: intent.id.clone(),
            candidate: intent.candidate.clone(),
            policy_digest: intent.policy_digest.clone(),
            job_id: Some(dispatched_job.id),
            review_id: Some(number(saved, "/id")?),
            reviewed_head: intent.candidate.head.clone(),
            reviewed_base: intent.candidate.base.clone(),
            status: "done".into(),
            complete_findings: true,
            document: Some(document),
        };
        receipt.validate(intent, policy)?;
        Ok(receipt)
    }

    fn validate_binding(&self, intent: &Intent, policy: &Policy) -> Result<(), Error> {
        self.candidate.validate()?;
        if self.schema_version != 1
            || self.request_id != intent.id
            || self.request_id.is_empty()
            || !same_comparison(&self.candidate, &intent.candidate)
            || self.policy_digest != intent.policy_digest
            || self.policy_digest != policy.digest()?
            || self.reviewed_head != intent.candidate.head
            || self.reviewed_base != intent.candidate.base
            || self.job_id == Some(0)
            || self.review_id == Some(0)
        {
            return Err(Error(
                "roborev receipt does not attest the exact request, comparison and policy".into(),
            ));
        }
        if !matches!(
            self.status.as_str(),
            "queued" | "running" | "done" | "failed" | "skipped" | "cancelled"
        ) {
            return Err(Error("unsupported roborev execution state".into()));
        }
        Ok(())
    }

    fn validate(&self, intent: &Intent, policy: &Policy) -> Result<(), Error> {
        self.validate_binding(intent, policy)?;
        if self.status != "done"
            || !self.complete_findings
            || self.job_id.is_none()
            || self.review_id.is_none()
        {
            return Err(Error(
                "roborev execution failed, skipped, pending or has incomplete findings".into(),
            ));
        }
        let document = self
            .document
            .as_ref()
            .ok_or_else(|| Error("roborev result lacks its complete canonical document".into()))?;
        if document.schema_version != 2
            || document.summary.trim().is_empty()
            || !matches!(document.verdict.as_str(), "pass" | "fail")
            || (document.verdict == "fail" && document.findings.is_empty())
            || document
                .source_labels
                .iter()
                .any(|label| label.trim().is_empty())
            || document.findings.iter().any(|finding| {
                !matches!(
                    finding.severity.as_str(),
                    "critical" | "high" | "medium" | "low"
                ) || finding.problem.trim().is_empty()
                    || finding.fix.trim().is_empty()
                    || finding
                        .sources
                        .iter()
                        .any(|index| *index == 0 || *index > document.source_labels.len() as u64)
            })
        {
            return Err(Error(
                "roborev canonical document is unsupported, invalid or unable to review".into(),
            ));
        }
        Ok(())
    }

    /// Encode one complete receipt for an authorized producer. Oversized or
    /// invalid evidence must not be truncated into an apparent clean review.
    pub fn comment(&self, intent: &Intent, policy: &Policy) -> Result<String, Error> {
        self.validate_binding(intent, policy)?;
        if self.status == "done" {
            self.validate(intent, policy)?;
        }
        let body = format!("{RESULT_MARKER}{} -->", serde_json::to_string(self)?);
        if body.len() > MAX_RECEIPT_BYTES {
            return Err(Error(
                "roborev receipt exceeds the supported complete-comment bound".into(),
            ));
        }
        Ok(body)
    }
}

/// Explicit Toolbelt coordinator protocol; stock roborev comments cannot qualify.
pub struct RoborevGitHub {
    pub(super) github: GitHub,
}

impl RoborevGitHub {
    /// Select the authenticated complete-receipt transport.
    pub fn new(github: GitHub) -> Self {
        Self { github }
    }
}

fn receipt_id(comment: &Value) -> Result<String, Error> {
    Ok(format!("roborev-comment-{}", number(comment, "/id")?))
}

impl Provider for RoborevGitHub {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            provider: "roborev".into(),
            transport: "github-receipt-v1".into(),
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
        if policy.provider != "roborev" || policy.transport != "github-receipt-v1" {
            return Err(Error(
                "roborev adapter requires an explicitly selected receipt policy".into(),
            ));
        }
        policy.validate()?;
        policy.policy_app_id()?;
        candidate.validate()?;
        let pr = PullRequest::parse(&candidate.url)?;
        let comments = self.github.pages(
            &format!("/repos/{}/issues/{}/comments", pr.repository, pr.number),
            None,
        )?;
        let mut permissions = BTreeMap::new();
        let mut recovered_intent = None;
        let mut request_comment = None;
        for comment in &comments {
            if let Some(intent) = marker::<Intent>(text(comment, "/body")?, REQUEST_MARKER) {
                if intent.matches(candidate, policy)
                    && local.is_none_or(|old| old.id == intent.id)
                    && self.github.authorized(&pr, comment, &mut permissions)?
                    && request_comment
                        .is_none_or(|old: &Value| comment["id"].as_u64() > old["id"].as_u64())
                {
                    number(comment, "/id")?;
                    request_comment = Some(comment);
                    recovered_intent = Some(intent);
                }
            }
        }
        let trusted = |comment: &&Value| {
            comment.pointer("/user/id").and_then(Value::as_u64) == Some(policy.reviewer_id)
                && comment.pointer("/user/type").and_then(Value::as_str) == Some("Bot")
                && comment
                    .get("body")
                    .and_then(Value::as_str)
                    .is_some_and(|body| body.starts_with(RESULT_MARKER))
        };
        let results: Vec<_> = comments.iter().filter(trusted).collect();
        let known_review_ids = results
            .iter()
            .map(|comment| receipt_id(comment))
            .collect::<Result<Vec<_>, _>>()?;
        let request_receipt = request_comment
            .map(|comment| text(comment, "/html_url").map(str::to_owned))
            .transpose()?;
        let mut review = None;
        if let (Some(intent), Some(request)) =
            (local.or(recovered_intent.as_ref()), request_comment)
        {
            let requested_at = timestamp(
                request
                    .get("updated_at")
                    .and_then(Value::as_str)
                    .unwrap_or(text(request, "/created_at")?),
            )?;
            let mut selected = None;
            for comment in results {
                let body = text(comment, "/body")?;
                if body.len() > MAX_RECEIPT_BYTES {
                    return Err(Error(
                        "oversized roborev receipt; complete findings cannot be established".into(),
                    ));
                }
                let json = body
                    .strip_prefix(RESULT_MARKER)
                    .and_then(|body| body.strip_suffix(" -->"))
                    .ok_or_else(|| Error("incomplete roborev receipt marker".into()))?;
                let result: RoborevReceipt = serde_json::from_str(json)
                    .map_err(|_| Error("malformed or unsupported roborev receipt".into()))?;
                let observed_at = timestamp(
                    comment
                        .get("updated_at")
                        .and_then(Value::as_str)
                        .unwrap_or(text(comment, "/created_at")?),
                )?;
                if result.request_id == intent.id
                    && !intent.baseline_review_ids.contains(&receipt_id(comment)?)
                    && timestamp(text(comment, "/created_at")?)? >= requested_at
                    && selected.as_ref().is_none_or(
                        |(old, _, at): &(&Value, RoborevReceipt, i64)| {
                            (observed_at, comment["id"].as_u64()) > (*at, old["id"].as_u64())
                        },
                    )
                {
                    selected = Some((comment, result, observed_at));
                }
            }
            if let Some((comment, result, _)) = selected {
                result.validate(intent, policy)?;
                let id = receipt_id(comment)?;
                let url = text(comment, "/html_url")?.to_owned();
                let mut findings = result
                    .document
                    .as_ref()
                    .ok_or_else(|| Error("missing canonical review document".into()))?
                    .findings
                    .iter()
                    .enumerate()
                    .map(|(index, finding)| {
                        let body = serde_json::to_string(finding)?;
                        Ok(Finding {
                            id: format!("{id}/{}-{}", index + 1, hex_digest(body.as_bytes())),
                            body,
                            path: finding.location.clone(),
                            line: None,
                            severity: Some(finding.severity.clone()),
                            provider_addressed: false,
                            correlated: true,
                            url: url.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                for comment in &comments {
                    if let Some(disposition) =
                        marker::<Disposition>(text(comment, "/body")?, DISPOSITION_MARKER)
                    {
                        if disposition.head == candidate.head
                            && disposition.base == candidate.base
                            && disposition.policy_digest == policy.digest()?
                            && disposition.review_id == id
                            && !disposition.reason.trim().is_empty()
                            && !disposition.evidence.trim().is_empty()
                            && self.github.authorized(&pr, comment, &mut permissions)?
                        {
                            findings.retain(|finding| {
                                finding.id != disposition.finding_id
                                    || hex_digest(finding.body.as_bytes())
                                        != disposition.body_digest
                            });
                        }
                    }
                }
                review = Some(Review {
                    id,
                    provider_job_id: result.job_id.map(|id| id.to_string()),
                    provider_review_id: result.review_id.map(|id| id.to_string()),
                    provider: "roborev".into(),
                    reviewed_head: Some(result.reviewed_head),
                    requested_base: Some(intent.candidate.base.clone()),
                    reviewed_base: Some(result.reviewed_base),
                    policy_digest: result.policy_digest,
                    completed: true,
                    complete_findings: true,
                    findings,
                    url,
                });
            }
        }
        Ok(Evidence {
            known_review_ids,
            recovered_intent,
            request_receipt,
            review,
        })
    }

    fn submit(&mut self, intent: &Intent) -> Result<String, Error> {
        intent.candidate.validate()?;
        let pr = PullRequest::parse(&intent.candidate.url)?;
        let body = format!(
            "{REQUEST_MARKER}{} -->\n\nRoborev coordinator request; exact comparison and policy are frozen in the intent.",
            serde_json::to_string(intent)?
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

#[cfg(test)]
mod tests {
    use super::super::tests::server;
    use super::*;

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

    fn intent() -> Intent {
        Intent {
            schema_version: 1,
            id: "fixture-request".into(),
            policy_digest: policy().digest().unwrap(),
            baseline_review_ids: vec![],
            candidate: Candidate {
                url: "https://github.com/example/project/pull/12".into(),
                source_repository: "example/fork".into(),
                source_branch: "feature".into(),
                target_branch: "main".into(),
                head: "a".repeat(40),
                base: "b".repeat(40),
                draft: false,
                open: true,
            },
        }
    }

    fn receipt() -> RoborevReceipt {
        let intent = intent();
        RoborevReceipt {
            schema_version: 1,
            request_id: intent.id,
            candidate: intent.candidate.clone(),
            policy_digest: intent.policy_digest,
            job_id: Some(7),
            review_id: Some(9),
            reviewed_head: intent.candidate.head,
            reviewed_base: intent.candidate.base,
            status: "done".into(),
            complete_findings: true,
            document: Some(RoborevDocument {
                schema_version: 2,
                summary: "fixture result".into(),
                verdict: "pass".into(),
                findings: vec![],
                source_labels: vec![],
            }),
        }
    }

    fn request() -> Value {
        json!({"id":12,"body":format!("{REQUEST_MARKER}{} -->", serde_json::to_string(&intent()).unwrap()),
            "html_url":"https://github.com/example/project/pull/12#issuecomment-12",
            "created_at":"2026-10-03T12:00:00Z", "user":{"login":"writer"}})
    }

    fn result(id: u64, receipt: &RoborevReceipt) -> Value {
        json!({"id":id,"body":format!("{RESULT_MARKER}{} -->",serde_json::to_string(receipt).unwrap()),
            "html_url":format!("https://github.com/example/project/pull/12#issuecomment-{id}"),
            "created_at":"2026-10-03T12:01:00Z", "user":{"id":123,"type":"Bot"}})
    }

    fn collect(comments: Vec<Value>) -> Result<Evidence, Error> {
        let (github, handle) = server(vec![
            (200, json!(comments)),
            (200, json!({"permission":"write"})),
        ]);
        let evidence = RoborevGitHub::new(github).inspect(&intent().candidate, &policy(), None);
        handle.join().unwrap();
        evidence
    }

    #[test]
    fn full_authenticated_canonical_result_qualifies() {
        let evidence = collect(vec![request(), result(19, &receipt())]).unwrap();
        let review = evidence.review.unwrap();
        assert_eq!(review.id, "roborev-comment-19");
        assert_eq!(review.provider_job_id.as_deref(), Some("7"));
        assert_eq!(review.provider_review_id.as_deref(), Some("9"));
        assert_eq!(review.reviewed_head, Some("a".repeat(40)));
        assert_eq!(review.reviewed_base, Some("b".repeat(40)));
        assert_eq!(
            policy().evaluate(&intent().candidate, &review),
            crate::review::Verdict::Ready
        );
        assert_eq!(evidence.recovered_intent.unwrap().id, intent().id);
    }

    #[test]
    fn producer_requires_actual_persisted_range_job_and_canonical_output() {
        let frozen = intent();
        let dispatch = crate::review::RoborevDispatch {
            intent: frozen.clone(),
            checkout: "/frozen/request".into(),
            agent: "opencode".into(),
            expected_files: 1,
        };
        let identity = crate::review::RoborevJobIdentity {
            id: 7,
            uuid: "11111111-1111-1111-1111-111111111111".into(),
        };
        let saved = json!({"id":9,"job_id":7,"agent":"opencode","structured_output":receipt().document,
            "job":{"id":7,"uuid":identity.uuid,"repo_path":"/frozen/request","git_ref":format!("{}..{}",frozen.candidate.base,frozen.candidate.head),
                "agent":"opencode","status":"done","job_type":"range","agentic":false,"prompt_prebuilt":false,"min_severity":"low"},
            "file_coverage":{"reviewed":1,"excluded":0}});
        let build = |value: &Value| {
            RoborevReceipt::from_saved_review(&dispatch, &policy(), &identity, value)
        };
        assert!(build(&saved).is_ok());
        for (pointer, value) in [
            ("/job/git_ref", json!(frozen.candidate.head)),
            ("/job/id", json!(8)),
            ("/job/uuid", json!("22222222-2222-2222-2222-222222222222")),
            ("/job_id", json!(8)),
            ("/job/repo_path", json!("/other/request")),
            ("/job/agent", json!("codex")),
            ("/job/status", json!("failed")),
            ("/job/job_type", json!("panel_synthesis")),
            ("/job/agentic", json!(true)),
            ("/job/prompt_prebuilt", json!(true)),
            ("/job/min_severity", json!("high")),
            ("/structured_output", Value::Null),
        ] {
            let mut invalid = saved.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            assert!(build(&invalid).is_err(), "accepted invalid field {pointer}");
        }
    }

    #[test]
    fn producer_rejects_unmeasured_or_excluded_comparisons() {
        let frozen = intent();
        let dispatch = crate::review::RoborevDispatch {
            intent: frozen.clone(),
            checkout: "/frozen/request".into(),
            agent: "opencode".into(),
            expected_files: 1,
        };
        let identity = crate::review::RoborevJobIdentity {
            id: 7,
            uuid: "11111111-1111-1111-1111-111111111111".into(),
        };
        let saved = json!({"id":9,"job_id":7,"agent":"opencode","structured_output":receipt().document,
            "job":{"id":7,"uuid":identity.uuid,"repo_path":"/frozen/request","git_ref":format!("{}..{}",frozen.candidate.base,frozen.candidate.head),
                "agent":"opencode","status":"done","job_type":"range","agentic":false,"prompt_prebuilt":false,"min_severity":"low"},
            "file_coverage":{"reviewed":1,"excluded":0}});
        for coverage in [
            Value::Null,
            json!({"reviewed":0,"excluded":4}),
            json!({"reviewed":0,"excluded":0}),
            json!({"reviewed":2,"excluded":0}),
            json!({"reviewed":-1,"excluded":0}),
            json!({"reviewed":1}),
            json!({"excluded":0}),
        ] {
            let mut invalid = saved.clone();
            invalid["file_coverage"] = coverage;
            assert!(
                RoborevReceipt::from_saved_review(&dispatch, &policy(), &identity, &invalid,)
                    .is_err(),
                "accepted incomplete coverage: {}",
                invalid["file_coverage"]
            );
        }
        let mut absent = saved.clone();
        absent.as_object_mut().unwrap().remove("file_coverage");
        assert!(
            RoborevReceipt::from_saved_review(&dispatch, &policy(), &identity, &absent).is_err()
        );
        let mut empty = dispatch.clone();
        empty.expected_files = 0;
        assert!(RoborevReceipt::from_saved_review(&empty, &policy(), &identity, &saved).is_err());
    }

    #[test]
    fn wrong_identity_and_stock_markdown_do_not_qualify() {
        let mut forged = result(19, &receipt());
        forged["user"]["id"] = json!(999);
        let mut human = result(20, &receipt());
        human["user"]["type"] = json!("User");
        let mut stock = result(21, &receipt());
        stock["body"] =
            json!("<!-- roborev-pr-comment -->\n## roborev: Pass (`aaaaaaa`)\nNo issues found.");
        assert!(
            collect(vec![request(), forged, human, stock])
                .unwrap()
                .review
                .is_none()
        );
    }

    #[test]
    fn every_comparison_and_execution_field_is_enforced() {
        let mutations: &[fn(&mut RoborevReceipt)] = &[
            |r| r.reviewed_head = "c".repeat(40),
            |r| r.reviewed_base = "c".repeat(40),
            |r| r.reviewed_head = "aaaaaaa".into(),
            |r| r.candidate.source_repository = "other/fork".into(),
            |r| r.candidate.target_branch = "other".into(),
            |r| r.policy_digest = "old-policy".into(),
            |r| r.job_id = Some(0),
            |r| r.review_id = Some(0),
            |r| r.job_id = None,
            |r| r.review_id = None,
            |r| r.document = None,
            |r| r.complete_findings = false,
            |r| r.status = "failed".into(),
            |r| r.status = "skipped".into(),
            |r| r.status = "cancelled".into(),
            |r| r.document.as_mut().unwrap().schema_version = 1,
            |r| r.document.as_mut().unwrap().verdict = "unable_to_review".into(),
            |r| r.document.as_mut().unwrap().verdict = "fail".into(),
        ];
        for mutation in mutations {
            let mut invalid = receipt();
            mutation(&mut invalid);
            assert!(collect(vec![request(), result(19, &invalid)]).is_err());
        }
    }

    #[test]
    fn latest_failed_receipt_cannot_revive_older_pass() {
        let mut failed = receipt();
        failed.status = "failed".into();
        failed.review_id = None;
        failed.document = None;
        failed.complete_findings = false;
        assert!(
            failed.comment(&intent(), &policy()).is_ok(),
            "failure receipt must not invent a review"
        );
        assert!(collect(vec![request(), result(19, &receipt()), result(20, &failed)]).is_err());
    }

    #[test]
    fn newer_failure_edit_cannot_revive_a_later_created_pass() {
        let mut failed = receipt();
        failed.status = "failed".into();
        failed.document = None;
        failed.review_id = None;
        failed.complete_findings = false;
        let mut edited = result(19, &failed);
        edited["updated_at"] = json!("2026-10-03T12:03:00Z");
        assert!(collect(vec![request(), edited, result(20, &receipt())]).is_err());
    }

    #[test]
    fn low_findings_block_even_when_agent_reports_pass() {
        let mut content = receipt();
        content
            .document
            .as_mut()
            .unwrap()
            .findings
            .push(RoborevFinding {
                severity: "low".into(),
                problem: "fixture problem".into(),
                fix: "fixture fix".into(),
                location: None,
                sources: vec![],
            });
        let evidence = collect(vec![request(), result(19, &content)]).unwrap();
        assert_eq!(
            policy().evaluate(&intent().candidate, &evidence.review.unwrap()),
            crate::review::Verdict::Findings
        );
    }

    #[test]
    fn missing_findings_location_and_truncated_receipts_block() {
        for field in ["findings", "location"] {
            let mut value = serde_json::to_value(receipt()).unwrap();
            if field == "findings" {
                value["document"].as_object_mut().unwrap().remove(field);
            } else {
                value["document"]["findings"] =
                    json!([{"severity":"high","problem":"problem","fix":"fix"}]);
            }
            let mut comment = result(19, &receipt());
            comment["body"] = json!(format!("{RESULT_MARKER}{value} -->"));
            assert!(collect(vec![request(), comment]).is_err());
        }
        let mut truncated = result(20, &receipt());
        truncated["body"] = json!(format!("{RESULT_MARKER}{{\"schemaVersion\":1"));
        assert!(collect(vec![request(), result(19, &receipt()), truncated]).is_err());
    }

    #[test]
    fn editing_request_does_not_adopt_preexisting_completion() {
        let mut edited = request();
        edited["updated_at"] = json!("2026-10-03T12:02:00Z");
        assert!(
            collect(vec![edited, result(19, &receipt())])
                .unwrap()
                .review
                .is_none()
        );
    }

    #[test]
    fn oversized_producer_receipt_is_rejected_instead_of_truncated() {
        let mut oversized = receipt();
        oversized.document.as_mut().unwrap().summary = "x".repeat(MAX_RECEIPT_BYTES);
        assert!(oversized.comment(&intent(), &policy()).is_err());
        assert!(collect(vec![request(), result(19, &oversized)]).is_err());
    }
}
