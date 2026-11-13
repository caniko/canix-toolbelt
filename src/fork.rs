//! Read-only fork/upstream divergence reporting.
//!
//! A fork is a checkout that carries local commits on top of an upstream
//! project. Reporting only inspects local Git state; fetching is opt-in and
//! nothing here rewrites refs, branches or worktrees.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Remote names treated as the upstream project, in preference order.
pub const UPSTREAM_REMOTES: [&str; 2] = ["upstream", "source"];

/// Branches probed for the upstream default when symbolic HEAD is absent.
pub const DEFAULT_BRANCHES: [&str; 5] = ["main", "master", "dev", "develop", "trunk"];

/// Divergence of one fork checkout from its upstream default branch.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkStatus {
    /// Directory name of the fork.
    pub name: String,
    /// Checked-out branch, absent for a detached HEAD.
    pub branch: Option<String>,
    /// Upstream remote used for comparison.
    pub upstream: Option<String>,
    /// Upstream ref compared against, for example `upstream/main`.
    pub upstream_ref: Option<String>,
    /// Commits reachable only from the upstream ref.
    pub behind: Option<u64>,
    /// Commits reachable only from the checkout.
    pub ahead: Option<u64>,
    /// Why divergence could not be measured.
    pub problem: Option<String>,
}

impl ForkStatus {
    fn problem(name: &str, problem: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            branch: None,
            upstream: None,
            upstream_ref: None,
            behind: None,
            ahead: None,
            problem: Some(problem.into()),
        }
    }

    /// Whether the fork is a checkout at all.
    pub fn is_checkout(&self) -> bool {
        self.problem.as_deref() != Some("not a Git checkout")
    }

    /// Whether this fork carries commits the upstream lacks.
    pub fn has_local_commits(&self) -> bool {
        self.ahead.unwrap_or_default() > 0
    }
}

fn git(dir: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|error| format!("git {arguments:?}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_owned())
        .map_err(|_| format!("git {arguments:?}: output is not UTF-8"))
}

/// Select the upstream remote from configured remote names.
pub fn upstream_remote(remotes: &[String]) -> Option<String> {
    UPSTREAM_REMOTES
        .iter()
        .find(|candidate| remotes.iter().any(|remote| remote == *candidate))
        .map(|candidate| (*candidate).to_owned())
}

/// Parse `git rev-list --left-right --count upstream...head` into (behind, ahead).
pub fn parse_divergence(counts: &str) -> Option<(u64, u64)> {
    let mut fields = counts.split_whitespace();
    let behind = fields.next()?.parse().ok()?;
    let ahead = fields.next()?.parse().ok()?;
    Some((behind, ahead))
}

/// Resolve the upstream branch ref: symbolic HEAD first, then known defaults.
pub fn upstream_ref(dir: &Path, remote: &str) -> Option<String> {
    let symbolic = git(
        dir,
        &[
            "symbolic-ref",
            "--short",
            &format!("refs/remotes/{remote}/HEAD"),
        ],
    )
    .ok()
    .filter(|reference| !reference.is_empty());
    if let Some(reference) = symbolic {
        return Some(reference);
    }
    DEFAULT_BRANCHES
        .iter()
        .map(|branch| format!("{remote}/{branch}"))
        .find(|reference| git(dir, &["rev-parse", "--verify", "--quiet", reference]).is_ok())
}

fn head_branch(dir: &Path) -> Option<String> {
    git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .filter(|branch| !branch.is_empty() && branch != "HEAD")
}

/// Inspect one fork checkout.
pub fn inspect(dir: &Path, fetch: bool) -> ForkStatus {
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.display().to_string());
    if !dir.join(".git").exists() {
        return ForkStatus::problem(&name, "not a Git checkout");
    }
    let remotes = match git(dir, &["remote"]) {
        Ok(output) => output
            .lines()
            .map(|line| line.trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>(),
        Err(error) => return ForkStatus::problem(&name, error),
    };
    let Some(upstream) = upstream_remote(&remotes) else {
        return ForkStatus::problem(&name, "no upstream remote");
    };
    if fetch {
        if let Err(error) = git(dir, &["fetch", "--quiet", &upstream]) {
            return ForkStatus {
                upstream: Some(upstream),
                ..ForkStatus::problem(&name, error)
            };
        }
    }
    let branch = head_branch(dir);
    let Some(reference) = upstream_ref(dir, &upstream) else {
        return ForkStatus {
            branch,
            upstream: Some(upstream),
            ..ForkStatus::problem(&name, "no upstream default branch")
        };
    };
    let counts = match git(
        dir,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{reference}...HEAD"),
        ],
    ) {
        Ok(counts) => counts,
        Err(error) => {
            return ForkStatus {
                branch,
                upstream: Some(upstream),
                upstream_ref: Some(reference),
                ..ForkStatus::problem(&name, error)
            };
        }
    };
    let Some((behind, ahead)) = parse_divergence(&counts) else {
        return ForkStatus {
            branch,
            upstream: Some(upstream),
            upstream_ref: Some(reference),
            ..ForkStatus::problem(&name, "unreadable divergence")
        };
    };
    ForkStatus {
        name,
        branch,
        upstream: Some(upstream),
        upstream_ref: Some(reference),
        behind: Some(behind),
        ahead: Some(ahead),
        problem: None,
    }
}

/// Enumerate and inspect every fork directory under `root`, sorted by name.
pub fn survey(root: &Path, fetch: bool) -> Result<Vec<ForkStatus>, String> {
    let entries =
        std::fs::read_dir(root).map_err(|error| format!("{}: {error}", root.display()))?;
    let mut directories = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .collect::<Vec<PathBuf>>();
    directories.sort();
    Ok(directories
        .into_iter()
        .map(|directory| inspect(&directory, fetch))
        .collect())
}

/// Render one status per line, aligned for reading in a terminal.
pub fn table(rows: &[ForkStatus]) -> String {
    let width = rows.iter().map(|row| row.name.len()).max().unwrap_or(0);
    let mut lines = Vec::with_capacity(rows.len());
    for row in rows {
        let position = match (row.ahead, row.behind) {
            (Some(ahead), Some(behind)) => format!("behind {behind:>6} ahead {ahead:>4}"),
            _ => "-".to_owned(),
        };
        let memory = row.problem.clone().unwrap_or_default();
        lines.push(format!(
            "{:<width$}  {:<24}  {}",
            row.name,
            position,
            memory,
            width = width
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_the_upstream_remote_by_preference() {
        let remotes = vec![
            "origin".to_owned(),
            "source".to_owned(),
            "upstream".to_owned(),
        ];
        assert_eq!(upstream_remote(&remotes).as_deref(), Some("upstream"));
        let remotes = vec!["origin".to_owned(), "source".to_owned()];
        assert_eq!(upstream_remote(&remotes).as_deref(), Some("source"));
        assert_eq!(upstream_remote(&["origin".to_owned()]), None);
    }

    #[test]
    fn reads_left_right_counts_as_behind_then_ahead() {
        assert_eq!(parse_divergence("964\t1"), Some((964, 1)));
        assert_eq!(parse_divergence("  12\t0\n"), Some((12, 0)));
        assert_eq!(parse_divergence("nonsense"), None);
    }

    #[test]
    fn renders_unknown_rows_without_counts() {
        let rows = vec![
            ForkStatus::problem("b", "no upstream remote"),
            ForkStatus {
                name: "a".to_owned(),
                branch: Some("main".to_owned()),
                upstream: Some("upstream".to_owned()),
                upstream_ref: Some("upstream/main".to_owned()),
                behind: Some(3),
                ahead: Some(1),
                problem: None,
            },
        ];
        let rendered = table(&rows);
        assert!(rendered.contains("behind      3 ahead    1"), "{rendered}");
        assert!(rendered.contains("no upstream remote"), "{rendered}");
    }
}
