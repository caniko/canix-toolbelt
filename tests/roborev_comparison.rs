#![cfg(feature = "review")]

use canix_toolbelt::review::{
    Candidate, CheckRequirement, Intent, Policy, RoborevDispatch, RoborevJobIdentity,
    RoborevReceipt,
};
use serde_json::json;
use std::{path::Path, process::Command};

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn divergent_history_attests_target_tip_instead_of_merge_base() {
    let directory = tempfile::tempdir().unwrap();
    let repo = directory.path();
    git(repo, &["init", "--quiet", "--initial-branch=main"]);
    git(repo, &["config", "user.name", "Disposable Review Fixture"]);
    git(repo, &["config", "user.email", "fixture@example.invalid"]);
    git(repo, &["config", "commit.gpgsign", "false"]);
    git(repo, &["config", "core.hooksPath", "/dev/null"]);
    std::fs::write(repo.join("feature.txt"), "ancestor\n").unwrap();
    git(repo, &["add", "feature.txt"]);
    git(repo, &["commit", "--quiet", "-m", "ancestor"]);
    let ancestor = git(repo, &["rev-parse", "HEAD"]);
    std::fs::write(repo.join("target-only.txt"), "target advancement\n").unwrap();
    git(repo, &["add", "target-only.txt"]);
    git(repo, &["commit", "--quiet", "-m", "advance target"]);
    let base = git(repo, &["rev-parse", "HEAD"]);
    git(repo, &["checkout", "--quiet", "-b", "feature", &ancestor]);
    std::fs::write(repo.join("feature.txt"), "feature advancement\n").unwrap();
    git(repo, &["add", "feature.txt"]);
    git(repo, &["commit", "--quiet", "-m", "advance feature"]);
    let head = git(repo, &["rev-parse", "HEAD"]);
    assert_eq!(git(repo, &["merge-base", &base, &head]), ancestor);
    assert_ne!(base, ancestor);
    assert_ne!(head, ancestor);
    let exact_range = format!("{base}..{head}");
    let merge_base_range = format!("{ancestor}..{head}");
    assert_eq!(
        git(repo, &["diff", "--name-only", &exact_range])
            .lines()
            .count(),
        2
    );
    assert_eq!(
        git(repo, &["diff", "--name-only", &merge_base_range])
            .lines()
            .count(),
        1
    );

    let policy = Policy {
        provider: "roborev".into(),
        transport: "github-receipt-v1".into(),
        reviewer_id: 123,
        required_checks: vec![CheckRequirement {
            name: "review-policy".into(),
            app_id: Some(456),
        }],
        ..Policy::default()
    };
    let intent = Intent {
        schema_version: 1,
        id: "divergent-comparison".into(),
        candidate: Candidate {
            url: "https://github.com/example/project/pull/1".into(),
            source_repository: "example/fork".into(),
            source_branch: "feature".into(),
            target_branch: "main".into(),
            head,
            base,
            draft: false,
            open: true,
        },
        policy_digest: policy.digest().unwrap(),
        baseline_review_ids: vec![],
    };
    let checkout = repo.to_str().unwrap();
    let dispatch = RoborevDispatch {
        intent: intent.clone(),
        checkout: checkout.into(),
        agent: "opencode".into(),
        expected_files: git(repo, &["diff", "--name-only", &exact_range])
            .lines()
            .count(),
    };
    let identity = RoborevJobIdentity {
        id: 7,
        uuid: "11111111-1111-1111-1111-111111111111".into(),
    };
    let mut saved = json!({"id":9,"job_id":7,"agent":"opencode",
        "job":{"id":7,"uuid":identity.uuid,"repo_path":checkout,"git_ref":exact_range,"agent":"opencode",
            "job_type":"range","status":"done","agentic":false,"prompt_prebuilt":false,"min_severity":"low"},
        "structured_output":{"schema_version":2,"summary":"Full target-to-head comparison","verdict":"pass","findings":[]},
        "file_coverage":{"reviewed":2,"excluded":0}});
    let receipt = RoborevReceipt::from_saved_review(&dispatch, &policy, &identity, &saved).unwrap();
    assert_eq!(receipt.reviewed_base, intent.candidate.base);
    assert_eq!(receipt.reviewed_head, intent.candidate.head);
    assert!(receipt.comment(&intent, &policy).is_ok());
    saved["job"]["git_ref"] = json!(merge_base_range);
    assert!(
        RoborevReceipt::from_saved_review(&dispatch, &policy, &identity, &saved).is_err(),
        "echoing the requested target must not qualify a persisted merge-base job"
    );
}
