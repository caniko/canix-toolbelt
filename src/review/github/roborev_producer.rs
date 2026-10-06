//! Trusted forge observation/publication boundary, separate from daemon execution.
use super::roborev::{RoborevGitHub, RoborevReceipt};
use super::*;
use crate::review::{
    RoborevDispatch, RoborevRunner, dispatch_roborev_once,
    engine::save,
    roborev_dispatch::{claim, read_ledger},
};
use std::path::Path;

/// Fresh open comparison and request authored by an authorized repository writer.
/// Constructed only by authoritative forge observation, not deserializable input.
#[derive(Clone, Debug, Serialize)]
pub struct AuthorizedRoborevRequest {
    intent: Intent,
    comment_id: u64,
    edited_at: i64,
}

impl AuthorizedRoborevRequest {
    /// Exact authorized frozen intent.
    pub fn intent(&self) -> &Intent {
        &self.intent
    }
    /// Bind a consumer-owned exclusive checkout and approved adapter.
    pub fn dispatch(
        &self,
        checkout: String,
        agent: String,
        expected_files: usize,
    ) -> RoborevDispatch {
        RoborevDispatch {
            intent: self.intent.clone(),
            checkout,
            agent,
            expected_files,
        }
    }
}

/// One bounded coordinator observation; it never loops or starts a daemon.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum RoborevProgress {
    /// No current writer-authorized request; no worker preparation occurred.
    Unrequested,
    /// One persisted job is queued/running. Resume the same request/checkout.
    Pending {
        /// Frozen request identity.
        request_id: String,
    },
    /// Complete persisted evidence was published or exactly reconciled.
    /// A failed receipt is still a publication, never an acceptance verdict.
    Published {
        /// Trusted forge comment URL.
        receipt_url: String,
        /// Canonical receipt, including explicit terminal execution failures.
        receipt: Box<RoborevReceipt>,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Publication {
    schema_version: u32,
    request_digest: String,
    body_digest: String,
    receipt: Option<String>,
}

impl RoborevGitHub {
    fn observe_request(
        &mut self,
        url: &str,
        policy: &Policy,
    ) -> Result<(Option<AuthorizedRoborevRequest>, Vec<Value>), Error> {
        policy.validate()?;
        policy.policy_app_id()?;
        if policy.provider != "roborev" || policy.transport != "github-receipt-v1" {
            return Err(Error(
                "trusted roborev producer requires its explicit receipt policy".into(),
            ));
        }
        let candidate = self.github.candidate(url)?;
        candidate.validate()?;
        if !candidate.open {
            return Err(Error(
                "closed comparison cannot authorize roborev dispatch".into(),
            ));
        }
        let pr = PullRequest::parse(&candidate.url)?;
        let comments = self.github.pages(
            &format!("/repos/{}/issues/{}/comments", pr.repository, pr.number),
            None,
        )?;
        let mut permissions = BTreeMap::new();
        let mut selected: Option<AuthorizedRoborevRequest> = None;
        for comment in &comments {
            if let Some(intent) = marker::<Intent>(text(comment, "/body")?, REQUEST_MARKER) {
                if intent.matches(&candidate, policy)
                    && !intent.id.is_empty()
                    && self.github.authorized(&pr, comment, &mut permissions)?
                {
                    let id = number(comment, "/id")?;
                    if selected.as_ref().is_none_or(|old| id > old.comment_id) {
                        selected = Some(AuthorizedRoborevRequest {
                            intent,
                            comment_id: id,
                            edited_at: timestamp(
                                comment
                                    .get("updated_at")
                                    .and_then(Value::as_str)
                                    .unwrap_or(text(comment, "/created_at")?),
                            )?,
                        });
                    }
                }
            }
        }
        if selected.is_some() {
            let fresh = self.github.candidate(url)?;
            if !fresh.open || !same_comparison(&candidate, &fresh) {
                return Err(Error(
                    "comparison changed during authorized request observation".into(),
                ));
            }
        }
        Ok((selected, comments))
    }

    /// Observe one exact current request without launching a job or writing to GitHub.
    pub fn authorized_request(
        &mut self,
        url: &str,
        policy: &Policy,
    ) -> Result<Option<AuthorizedRoborevRequest>, Error> {
        self.observe_request(url, policy)
            .map(|(request, _)| request)
    }

    /// Discover authorization, prepare the consumer's exclusive immutable checkout,
    /// revalidate authorization, dispatch/collect once, then revalidate and publish.
    /// Preparation must not enqueue reviews or launch agents. Its runner must
    /// verify the actual checkout, full file census and qualified worker policy on
    /// every dispatch.
    /// Consumer-owned confinement/credential separation remain mandatory; this
    /// orchestration does not make a personal checkout or same-UID daemon safe.
    /// Errors retain the underlying UNKNOWN ledgers and must not be retried through
    /// a new checkout/state identity. No live operation is implicit in construction.
    pub fn coordinate_request_once<R: RoborevRunner>(
        &mut self,
        url: &str,
        policy: &Policy,
        state_dir: &Path,
        prepare: impl FnOnce(&AuthorizedRoborevRequest) -> Result<(String, String, usize, R), Error>,
    ) -> Result<RoborevProgress, Error> {
        let Some(request) = self.authorized_request(url, policy)? else {
            return Ok(RoborevProgress::Unrequested);
        };
        let (checkout, agent, expected_files, mut runner) = prepare(&request)?;
        let dispatch = request.dispatch(checkout, agent, expected_files);
        runner.verify_checkout(&dispatch)?;
        let current = self.authorized_request(url, policy)?;
        if current
            .as_ref()
            .map(serde_json::to_vec)
            .transpose()?
            .as_deref()
            != Some(serde_json::to_vec(&request)?.as_slice())
        {
            return Err(Error(
                "authorized request changed during worker preparation; no dispatch permitted"
                    .into(),
            ));
        }
        let Some(receipt) =
            dispatch_roborev_once(&dispatch, policy, &state_dir.join("dispatch"), &mut runner)?
        else {
            return Ok(RoborevProgress::Pending {
                request_id: request.intent.id,
            });
        };
        let receipt_url =
            self.publish_receipt_once(&request, policy, &receipt, &state_dir.join("publication"))?;
        Ok(RoborevProgress::Published {
            receipt_url,
            receipt: Box::new(receipt),
        })
    }

    /// Revalidate the authorized request and comparison, then publish one complete
    /// receipt. UNKNOWN is persisted before the POST; retries may reconcile an
    /// exact trusted comment but never replay an unacknowledged publication.
    /// The caller must obtain this result from current persisted daemon evidence.
    pub fn publish_receipt_once(
        &mut self,
        request: &AuthorizedRoborevRequest,
        policy: &Policy,
        receipt: &RoborevReceipt,
        state_dir: &Path,
    ) -> Result<String, Error> {
        let body = receipt.comment(&request.intent, policy)?;
        let (current, comments) = self.observe_request(&request.intent.candidate.url, policy)?;
        let request_digest = hex_digest(&serde_json::to_vec(request)?);
        if current
            .as_ref()
            .map(serde_json::to_vec)
            .transpose()?
            .as_deref()
            != Some(serde_json::to_vec(request)?.as_slice())
        {
            return Err(Error(
                "authorized request was edited, removed or superseded before receipt publication"
                    .into(),
            ));
        }
        let body_digest = hex_digest(body.as_bytes());
        // One lifetime per original request: payload and edit changes cannot
        // evade its unresolved or completed publication binding.
        let key = format!(
            "publication-v2-{}",
            hex_digest(&serde_json::to_vec(&(
                &request.intent.candidate.url,
                request.comment_id,
            ))?)
        );
        if state_dir.exists() {
            for entry in std::fs::read_dir(state_dir)? {
                let path = entry?.path();
                if path.extension().is_some_and(|e| e == "lock")
                    && !path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| s.starts_with("publication-v2-"))
                {
                    return Err(Error("legacy payload-scoped publication history requires operator reconciliation; no fresh publication is authorized".into()));
                }
            }
        }
        let (_lock, fresh, path) = claim(state_dir, &key)?;
        let old = read_ledger::<Publication>(&path, fresh)?;
        if old.as_ref().is_some_and(|old| {
            old.schema_version != 1
                || old.request_digest != request_digest
                || old.body_digest != body_digest
        }) {
            return Err(Error(
                "publication binding changed; reconcile the original frozen receipt without reposting".into(),
            ));
        }
        let trusted = |comment: &Value| -> Result<bool, Error> {
            Ok(
                comment.pointer("/user/id").and_then(Value::as_u64) == Some(policy.reviewer_id)
                    && comment.pointer("/user/type").and_then(Value::as_str) == Some("Bot")
                    && comment.get("body").and_then(Value::as_str) == Some(body.as_str())
                    && !request
                        .intent
                        .baseline_review_ids
                        .contains(&format!("roborev-comment-{}", number(comment, "/id")?))
                    && timestamp(text(comment, "/created_at")?)? >= request.edited_at,
            )
        };
        let mut matching = None;
        for comment in &comments {
            if trusted(comment)? {
                matching = Some(text(comment, "/html_url")?.to_owned());
            }
        }
        let mut journal = Publication {
            schema_version: 1,
            request_digest,
            body_digest,
            receipt: matching.clone(),
        };
        if let Some(url) = matching {
            save(&path, &journal)?;
            return Ok(url);
        }
        if old.is_some() {
            return Err(Error("receipt publication is unknown or its original comment disappeared; reconcile without another POST".into()));
        }
        save(&path, &journal)?;
        let pr = PullRequest::parse(&request.intent.candidate.url)?;
        let (_, comment) = self.github.request(
            "POST",
            &format!("/repos/{}/issues/{}/comments", pr.repository, pr.number),
            Some(json!({"body":body})),
        )?;
        if !trusted(&comment)? {
            return Err(Error(
                "GitHub did not confirm the exact receipt under its configured producer identity"
                    .into(),
            ));
        }
        let url = text(&comment, "/html_url")?.to_owned();
        journal.receipt = Some(url.clone());
        save(&path, &journal)?;
        Ok(url)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::server;
    use super::*;
    use crate::review::{CheckRequirement, RoborevDocument};
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
    fn candidate() -> Candidate {
        Candidate {
            url: "https://github.com/example/project/pull/12".into(),
            source_repository: "example/fork".into(),
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
            policy_digest: policy().digest().unwrap(),
            baseline_review_ids: vec![],
        }
    }
    fn candidate_json() -> Value {
        let c = candidate();
        json!({"html_url":c.url,"head":{"repo":{"full_name":c.source_repository},"ref":c.source_branch,"sha":c.head},"base":{"ref":c.target_branch,"sha":c.base},"draft":false,"state":"open"})
    }
    fn request() -> Value {
        json!({"id":12,"body":format!("{REQUEST_MARKER}{} -->",serde_json::to_string(&intent()).unwrap()),"user":{"login":"writer"},"created_at":"2026-10-03T12:00:00Z"})
    }
    fn authorized() -> AuthorizedRoborevRequest {
        AuthorizedRoborevRequest {
            intent: intent(),
            comment_id: 12,
            edited_at: timestamp("2026-10-03T12:00:00Z").unwrap(),
        }
    }
    fn receipt() -> RoborevReceipt {
        RoborevReceipt {
            schema_version: 1,
            request_id: intent().id,
            candidate: candidate(),
            policy_digest: policy().digest().unwrap(),
            job_id: Some(7),
            review_id: Some(9),
            reviewed_head: candidate().head,
            reviewed_base: candidate().base,
            status: "done".into(),
            complete_findings: true,
            document: Some(RoborevDocument {
                schema_version: 2,
                summary: "fixture".into(),
                verdict: "pass".into(),
                findings: vec![],
                source_labels: vec![],
            }),
        }
    }
    fn result() -> Value {
        json!({"id":19,"body":receipt().comment(&intent(),&policy()).unwrap(),"user":{"id":123,"type":"Bot"},"created_at":"2026-10-03T12:01:00Z","html_url":"https://github.com/example/project/pull/12#issuecomment-19"})
    }
    fn observations(comments: Vec<Value>) -> Vec<(u16, Value)> {
        vec![
            (200, candidate_json()),
            (200, json!(comments)),
            (200, json!({"permission":"write"})),
            (200, candidate_json()),
        ]
    }

    #[test]
    fn authorized_request_requires_fresh_comparison_and_repository_writer() {
        let (github, handle) = server(observations(vec![request()]));
        let selected = RoborevGitHub::new(github)
            .authorized_request(&candidate().url, &policy())
            .unwrap()
            .unwrap();
        handle.join().unwrap();
        assert_eq!(selected.intent().id, intent().id);
        let (github, handle) = server(vec![
            (200, candidate_json()),
            (200, json!([request()])),
            (200, json!({"permission":"read"})),
        ]);
        assert!(
            RoborevGitHub::new(github)
                .authorized_request(&candidate().url, &policy())
                .unwrap()
                .is_none()
        );
        handle.join().unwrap();
        let mut moved = candidate_json();
        moved["base"]["sha"] = json!("c".repeat(40));
        let mut responses = observations(vec![request()]);
        responses[3] = (200, moved);
        let (github, handle) = server(responses);
        assert!(
            RoborevGitHub::new(github)
                .authorized_request(&candidate().url, &policy())
                .is_err()
        );
        handle.join().unwrap();
    }

    #[test]
    fn unknown_publication_reconciles_exact_comment_without_reposting() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        let mut responses = observations(vec![request()]);
        responses.push((500, json!({"secret":"omitted"})));
        let (github, handle) = server(responses);
        assert!(
            RoborevGitHub::new(github)
                .publish_receipt_once(&authorized(), &policy(), &receipt(), &state)
                .is_err()
        );
        handle.join().unwrap();
        let (github, handle) = server(observations(vec![request()]));
        assert!(
            RoborevGitHub::new(github)
                .publish_receipt_once(&authorized(), &policy(), &receipt(), &state)
                .is_err()
        );
        handle.join().unwrap();
        let (github, handle) = server(observations(vec![request(), result()]));
        assert!(
            RoborevGitHub::new(github)
                .publish_receipt_once(&authorized(), &policy(), &receipt(), &state)
                .unwrap()
                .ends_with("issuecomment-19")
        );
        handle.join().unwrap();
    }

    #[test]
    fn changed_request_or_untrusted_publication_cannot_qualify() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        let mut edited = request();
        edited["updated_at"] = json!("2026-10-03T12:02:00Z");
        let (github, handle) = server(observations(vec![edited]));
        assert!(
            RoborevGitHub::new(github)
                .publish_receipt_once(&authorized(), &policy(), &receipt(), &state)
                .is_err()
        );
        handle.join().unwrap();
        let mut wrong = result();
        wrong["user"]["id"] = json!(999);
        let mut responses = observations(vec![request()]);
        responses.push((201, wrong));
        let (github, handle) = server(responses);
        assert!(
            RoborevGitHub::new(github)
                .publish_receipt_once(&authorized(), &policy(), &receipt(), &state)
                .is_err()
        );
        handle.join().unwrap();
        let (github, handle) = server(observations(vec![request(), result()]));
        assert!(
            RoborevGitHub::new(github)
                .publish_receipt_once(&authorized(), &policy(), &receipt(), &state)
                .is_ok()
        );
        handle.join().unwrap();
    }

    #[test]
    fn changed_receipt_cannot_bypass_an_existing_publication_attempt() {
        for post_reply in [500, 201] {
            let directory = tempfile::tempdir().unwrap();
            let state = directory.path().join("state");
            let mut responses = observations(vec![request()]);
            responses.push((post_reply, result()));
            let (github, handle) = server(responses);
            let first = RoborevGitHub::new(github).publish_receipt_once(
                &authorized(),
                &policy(),
                &receipt(),
                &state,
            );
            handle.join().unwrap();
            assert_eq!(first.is_ok(), post_reply == 201);

            let mut changed = receipt();
            changed.status = "failed".into();
            changed.complete_findings = false;
            changed.document = None;
            let (github, handle) = server(observations(vec![request()]));
            let error = RoborevGitHub::new(github)
                .publish_receipt_once(&authorized(), &policy(), &changed, &state)
                .unwrap_err();
            handle.join().unwrap();
            assert!(
                error.to_string().contains("publication binding changed"),
                "{error}"
            );
            assert_eq!(
                std::fs::read_dir(&state)
                    .unwrap()
                    .filter(|entry| entry
                        .as_ref()
                        .unwrap()
                        .path()
                        .extension()
                        .is_some_and(|e| e == "lock"))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn editing_request_identity_cannot_open_another_publication_lifetime() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        let mut responses = observations(vec![request()]);
        responses.push((500, json!({})));
        let (github, handle) = server(responses);
        assert!(
            RoborevGitHub::new(github)
                .publish_receipt_once(&authorized(), &policy(), &receipt(), &state)
                .is_err()
        );
        handle.join().unwrap();

        let mut edited = authorized();
        edited.intent.id = "edited-request-id".into();
        edited.edited_at += 60;
        let mut comment = request();
        comment["body"] = json!(format!(
            "{REQUEST_MARKER}{} -->",
            serde_json::to_string(&edited.intent).unwrap()
        ));
        comment["updated_at"] = json!("2026-10-03T12:01:00Z");
        let mut changed = receipt();
        changed.request_id = edited.intent.id.clone();
        let (github, handle) = server(observations(vec![comment]));
        let error = RoborevGitHub::new(github)
            .publish_receipt_once(&edited, &policy(), &changed, &state)
            .unwrap_err();
        handle.join().unwrap();
        assert!(
            error.to_string().contains("publication binding changed"),
            "{error}"
        );
    }

    #[test]
    fn missing_legacy_publication_history_cannot_authorize_a_new_post() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        std::fs::create_dir(&state).unwrap();
        std::fs::write(state.join(format!("{}.lock", "a".repeat(64))), "").unwrap();
        let (github, handle) = server(observations(vec![request()]));
        let error = RoborevGitHub::new(github)
            .publish_receipt_once(&authorized(), &policy(), &receipt(), &state)
            .unwrap_err();
        handle.join().unwrap();
        assert!(error.to_string().contains("legacy payload-scoped"));
    }

    struct CoordinatorRunner {
        enqueues: std::rc::Rc<std::cell::Cell<usize>>,
        jobs: Vec<Value>,
    }

    impl crate::review::RoborevRunner for CoordinatorRunner {
        fn verify_checkout(&mut self, _: &RoborevDispatch) -> Result<(), Error> {
            Ok(()) // Only a synthetic runner; no production-boundary assertion.
        }
        fn jobs(&mut self, _: &RoborevDispatch) -> Result<Vec<Value>, Error> {
            Ok(self.jobs.clone())
        }
        fn enqueue(
            &mut self,
            dispatch: &RoborevDispatch,
        ) -> Result<crate::review::RoborevJobIdentity, Error> {
            self.enqueues.set(self.enqueues.get() + 1);
            let identity = crate::review::RoborevJobIdentity {
                id: 7,
                uuid: "11111111-1111-1111-1111-111111111111".into(),
            };
            self.jobs.push(json!({"id":7,"uuid":identity.uuid,"repo_path":dispatch.checkout,
                "git_ref":format!("{}..{}",dispatch.intent.candidate.base,dispatch.intent.candidate.head),
                "agent":dispatch.agent,"status":"done","job_type":"range","agentic":false,
                "prompt_prebuilt":false,"min_severity":"low"}));
            Ok(identity)
        }
        fn saved_review(&mut self, id: u64) -> Result<Value, Error> {
            Ok(
                json!({"id":9,"job_id":id,"agent":"opencode","job":self.jobs[0],
                "file_coverage":{"reviewed":1,"excluded":0},
                "structured_output":receipt().document}),
            )
        }
    }

    #[test]
    fn coordinator_refuses_unrequested_work_before_preparation() {
        let (github, handle) = server(vec![
            (200, candidate_json()),
            (200, json!([request()])),
            (200, json!({"permission":"read"})),
        ]);
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        let progress = RoborevGitHub::new(github)
            .coordinate_request_once(
                &candidate().url,
                &policy(),
                &state,
                |_| -> Result<(String, String, usize, CoordinatorRunner), Error> {
                    panic!("unauthorized request reached worker preparation")
                },
            )
            .unwrap();
        assert!(matches!(progress, RoborevProgress::Unrequested));
        assert!(!state.exists());
        handle.join().unwrap();
    }

    #[test]
    fn coordinator_revalidates_edited_authorization_before_enqueue() {
        let mut responses = observations(vec![request()]);
        let mut edited = request();
        edited["updated_at"] = json!("2026-10-03T12:02:00Z");
        responses.extend(observations(vec![edited]));
        let (github, handle) = server(responses);
        let directory = tempfile::tempdir().unwrap();
        let enqueues = std::rc::Rc::new(std::cell::Cell::new(0));
        let runner = CoordinatorRunner {
            enqueues: enqueues.clone(),
            jobs: vec![],
        };
        assert!(
            RoborevGitHub::new(github)
                .coordinate_request_once(
                    &candidate().url,
                    &policy(),
                    &directory.path().join("state"),
                    |_| Ok(("/fixture/exclusive".into(), "opencode".into(), 1, runner)),
                )
                .is_err()
        );
        assert_eq!(enqueues.get(), 0);
        handle.join().unwrap();
    }

    #[test]
    fn coordinator_publishes_only_current_persisted_receipt_and_reconciles() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        let enqueues = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut runner = CoordinatorRunner {
            enqueues: enqueues.clone(),
            jobs: vec![],
        };
        let mut responses = observations(vec![request()]);
        responses.extend(observations(vec![request()]));
        responses.extend(observations(vec![request()]));
        responses.push((201, result()));
        let (github, handle) = server(responses);
        let progress = RoborevGitHub::new(github)
            .coordinate_request_once(&candidate().url, &policy(), &state, |_| {
                Ok((
                    "/fixture/exclusive".into(),
                    "opencode".into(),
                    1,
                    &mut runner,
                ))
            })
            .unwrap();
        assert!(
            matches!(progress, RoborevProgress::Published { receipt_url, .. } if receipt_url.ends_with("issuecomment-19"))
        );
        handle.join().unwrap();
        let mut responses = observations(vec![request(), result()]);
        responses.extend(observations(vec![request(), result()]));
        responses.extend(observations(vec![request(), result()]));
        let (github, handle) = server(responses);
        let again = RoborevGitHub::new(github)
            .coordinate_request_once(&candidate().url, &policy(), &state, |_| {
                Ok((
                    "/fixture/exclusive".into(),
                    "opencode".into(),
                    1,
                    &mut runner,
                ))
            })
            .unwrap();
        assert!(matches!(again, RoborevProgress::Published { .. }));
        assert_eq!(enqueues.get(), 1);
        handle.join().unwrap();
    }
}
