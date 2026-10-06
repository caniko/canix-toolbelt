//! Offline qualification bridge; never connects to a forge or supplies credentials.
#[cfg(all(feature = "review", unix))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use canix_toolbelt::review::{
        Candidate, CheckRequirement, Error, Intent, Policy, RoborevDispatch, RoborevUnix,
        dispatch_roborev_once,
    };
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        time::{Duration, Instant},
    };
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        candidate: Candidate,
        checkout: String,
        socket: PathBuf,
        state: PathBuf,
    }
    let input: Input = serde_json::from_slice(&fs::read(
        std::env::args_os()
            .nth(1)
            .ok_or("fixture input path required")?,
    )?)?;
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
        id: "offline-producer".into(),
        candidate: input.candidate,
        policy_digest: policy.digest()?,
        baseline_review_ids: vec![],
    };
    let dispatch = RoborevDispatch {
        intent,
        checkout: input.checkout,
        agent: "opencode".into(),
    };
    let mut runner = RoborevUnix::new(
        input.socket,
        Duration::from_secs(10),
        |selected: &RoborevDispatch| {
            let git = |arguments: &[&str]| -> Result<String, Error> {
                let result = Command::new("git")
                    .arg("-C")
                    .arg(&selected.checkout)
                    .args(arguments)
                    .output()?;
                if !result.status.success() {
                    return Err(Error("offline fixture git verification failed".into()));
                }
                String::from_utf8(result.stdout)
                    .map_err(|_| Error("invalid fixture git output".into()))
            };
            if fs::canonicalize(&selected.checkout)?
                != Path::new(git(&["rev-parse", "--show-toplevel"])?.trim())
                || !git(&["status", "--porcelain", "--untracked-files=all"])?
                    .trim()
                    .is_empty()
                || Path::new(&selected.checkout).join(".roborev.toml").exists()
            {
                return Err(Error(
                    "fixture requires its clean exclusive checkout without repository settings"
                        .into(),
                ));
            }
            for commit in [
                &selected.intent.candidate.head,
                &selected.intent.candidate.base,
            ] {
                if git(&["rev-parse", "--verify", &format!("{commit}^{{commit}}")])?.trim()
                    != commit
                {
                    return Err(Error("fixture full comparison commit mismatch".into()));
                }
            }
            Ok(())
        },
    )?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(receipt) = dispatch_roborev_once(&dispatch, &policy, &input.state, &mut runner)?
        {
            // Validate the exact complete transport body without publishing it.
            receipt.comment(&dispatch.intent, &policy)?;
            println!("{}", serde_json::to_string(&receipt)?);
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("offline fixture deadline exceeded".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(not(all(feature = "review", unix)))]
fn main() {
    eprintln!("roborev fixture requires the review feature and Unix");
    std::process::exit(2);
}
