//! Source-owned custody policy checks; actionlint separately validates YAML.
use std::collections::BTreeSet;

const WORKFLOW: &str = include_str!("../.github/workflows/retain-producer-evidence.yaml");
const CI: &str = include_str!("../.github/workflows/ci.yaml");
const BUILDS: &str = include_str!("../.github/workflows/nix-builds.yaml");

fn spec() -> serde_json::Value {
    serde_json::from_str(include_str!("../.github/original-producers.json")).unwrap()
}

fn steps() -> Vec<&'static str> {
    WORKFLOW.split("      - ").skip(1).collect()
}

#[test]
fn original_enrollment_covers_every_generated_ci_and_matrix_job() {
    let spec = spec();
    assert_eq!(spec["schema"], "simit.original-observer-spec.v1");
    assert_eq!(spec["repository"], "caniko/canix-toolbelt");
    let jobs = CI.split_once("\njobs:\n").unwrap().1;
    let names: BTreeSet<_> = jobs
        .lines()
        .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
        .map(|line| line.trim().strip_suffix(':').unwrap().to_owned())
        .collect();
    let installables: BTreeSet<_> = BUILDS
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- \".#"))
        .map(|line| format!("build (.#{})", line.strip_suffix('"').unwrap()))
        .collect();
    assert_eq!(names.len(), 6);
    assert_eq!(installables.len(), 34);
    assert_eq!(spec["workflows"].as_object().unwrap().len(), 2);
    for (path, expected) in [
        (".github/workflows/ci.yaml", names),
        (".github/workflows/nix-builds.yaml", installables),
    ] {
        let enrolled = spec["workflows"][path].as_array().unwrap();
        let actual: BTreeSet<_> = enrolled
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            actual.len(),
            enrolled.len(),
            "duplicate enrollment in {path}"
        );
        assert_eq!(actual, expected, "incomplete enrollment in {path}");
    }
    assert!(CI.contains("run: nix flake check\n"));
}

#[test]
fn native_custody_reserves_finalization_and_diagnostics_inside_six_hours() {
    let minutes: Vec<u64> = WORKFLOW
        .lines()
        .filter_map(|line| line.trim().strip_prefix("timeout-minutes: "))
        .map(|value| value.parse().unwrap())
        .collect();
    assert_eq!(minutes, [360, 2, 1, 5, 20, 318, 3, 6, 1]);
    assert_eq!(minutes[1..].iter().sum::<u64>(), 356);
    assert_eq!(spec()["timeout_seconds"], 18_900);
    assert_eq!(spec()["poll_seconds"], 60);
    assert_eq!(
        minutes[5] * 60 - spec()["timeout_seconds"].as_u64().unwrap(),
        180
    );
    assert!(WORKFLOW.contains("cancel-in-progress: false"));
}

#[test]
fn native_custody_scopes_credentials_and_keeps_preflight_failures_separate() {
    let steps = steps();
    assert!(
        !WORKFLOW
            .split("    steps:")
            .next()
            .unwrap()
            .contains("GH_TOKEN")
    );
    assert!(steps[0].contains("ref: ${{ github.sha }}"));
    assert!(steps[0].contains("fetch-depth: 2"));
    assert!(steps[0].contains("persist-credentials: false"));
    assert!(steps[1].contains("git checkout --detach \"$SOURCE_HEAD\""));
    let provision = steps
        .iter()
        .find(|step| step.contains("SIMIT_OBSERVER_REVISION:"))
        .unwrap();
    let revision = provision
        .lines()
        .find_map(|line| line.trim().strip_prefix("SIMIT_OBSERVER_REVISION: "))
        .unwrap();
    assert_eq!(revision, "6f4f60c96cb79ac81863c05d7c7c6025f34a7dd4");
    assert!(provision.contains("github:caniko/simit/$SIMIT_OBSERVER_REVISION#default"));
    assert!(provision.contains("--no-write-lock-file --option access-tokens \"\""));
    assert_eq!(WORKFLOW.matches("GH_TOKEN:").count(), 2);
    for command in ["ci observe-original", "ci verify-original-retention"] {
        let step = steps.iter().find(|step| step.contains(command)).unwrap();
        assert!(step.contains("GH_TOKEN: ${{ github.token }}"));
        for variable in [
            "LD_PRELOAD",
            "LD_AUDIT",
            "LD_LIBRARY_PATH",
            "DYLD_INSERT_LIBRARIES",
            "DYLD_LIBRARY_PATH",
            "DYLD_FRAMEWORK_PATH",
            "DYLD_FALLBACK_LIBRARY_PATH",
            "DYLD_FALLBACK_FRAMEWORK_PATH",
            "BASH_ENV",
            "ENV",
        ] {
            assert!(
                step.lines()
                    .any(|line| line.trim() == format!("{variable}: \"\"")),
                "{command}: {variable}"
            );
        }
    }
    assert!(!WORKFLOW.contains("python"));
    assert!(!WORKFLOW.contains("GITHUB_ENV"));
    let upload = steps
        .iter()
        .find(|step| step.contains("id: upload"))
        .unwrap();
    assert!(upload.contains("if: always()"));
    assert!(upload.contains("if-no-files-found: error"));
    assert!(upload.contains("retention-days: 32"));
    assert!(upload.contains("path: ${{ runner.temp }}/producer-evidence/\n"));
    assert!(!upload.contains(".preflight-"));
    let diagnostics = steps.last().unwrap();
    assert!(diagnostics.contains("if: always()"));
    assert!(diagnostics.contains("if-no-files-found: warn"));
    assert!(diagnostics.contains("retention-days: 32"));
    for path in [
        "producer-evidence.retention/",
        "producer-evidence.preflight-*/",
        "producer-evidence.retention.preflight-*/",
    ] {
        assert!(diagnostics.contains(path), "missing diagnostic path {path}");
    }
}
