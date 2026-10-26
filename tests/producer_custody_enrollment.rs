//! Completion-driven custody must enroll every generated CI job and Nix build group.
const SIMIT: &str = include_str!("../simit.toml");
const CI: &str = include_str!("../.github/workflows/ci.yaml");
const BUILDS: &str = include_str!("../.github/workflows/nix-builds.yaml");

#[test]
fn custody_enrolls_every_ci_job_and_build_group() {
    let ci_jobs: Vec<_> = CI
        .split_once("\njobs:\n")
        .unwrap()
        .1
        .lines()
        .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
        .map(|line| line.trim().strip_suffix(':').unwrap())
        .collect();
    let groups: Vec<_> = BUILDS
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- group: \""))
        .map(|line| line.strip_suffix('"').unwrap())
        .collect();
    assert!(!ci_jobs.is_empty() && !groups.is_empty());
    let custody = SIMIT
        .split_once("[ci.custody]")
        .unwrap()
        .1
        .split_once("[ci.pages]")
        .unwrap()
        .0;
    for job in ci_jobs {
        assert!(
            custody.contains(&format!("\"{job}\"")),
            "unenrolled CI job {job}"
        );
    }
    for (index, group) in groups.iter().enumerate() {
        assert!(
            custody.contains(&format!("\"build ({group})\"")),
            "unenrolled group {group}"
        );
        assert!(
            custody.contains(&format!("\"nix-build-{index}\"")),
            "unenrolled artifact {index}"
        );
    }
}
