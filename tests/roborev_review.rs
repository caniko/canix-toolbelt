#![cfg(feature = "review")]

use canix_toolbelt::review::{CheckRequirement, GitHub, Policy, github_provider};

fn roborev_policy() -> Policy {
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

#[test]
fn provider_selection_is_explicit_and_never_falls_back() {
    let github = || GitHub::new("fixture-token".into()).unwrap();
    let policy = roborev_policy();
    let provider = github_provider(github(), &policy).unwrap();
    assert_eq!(provider.capabilities().provider, "roborev");
    assert_eq!(provider.capabilities().transport, "github-receipt-v1");
    let mut unsupported = policy.clone();
    unsupported.transport = "github-comment".into();
    assert!(github_provider(github(), &unsupported).is_err());
    unsupported = policy;
    unsupported.provider = "unknown".into();
    assert!(github_provider(github(), &unsupported).is_err());
    assert_eq!(
        github_provider(github(), &Policy::default())
            .unwrap()
            .capabilities()
            .provider,
        "greptile"
    );
}

#[test]
fn policy_gate_requires_one_exact_dedicated_app_identity() {
    let mut policy = roborev_policy();
    assert_eq!(policy.policy_app_id().unwrap(), 456);
    for app in [None, Some(0), Some(15368)] {
        policy.required_checks[0].app_id = app;
        assert!(policy.policy_app_id().is_err());
    }
    policy = roborev_policy();
    policy.required_checks.push(CheckRequirement {
        name: "review-policy".into(),
        app_id: Some(789),
    });
    assert!(policy.policy_app_id().is_err());
    policy.required_checks.clear();
    assert!(policy.policy_app_id().is_err());
}
