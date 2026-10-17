//! Private host-to-supervisor policy. Producer settings are applied only after
//! namespace, filesystem, descriptor and privilege containment is established.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

pub const MAX_LOG_BYTES: u64 = 512 * 1024 * 1024;
pub const SYSTEM_PATHS: &[&str] = &[
    "/nix/store",
    "/bin",
    "/usr/bin",
    "/usr/lib",
    "/usr/lib64",
    "/lib",
    "/lib64",
    "/run/current-system/sw",
    "/dev/null",
    "/dev/zero",
    "/dev/random",
    "/dev/urandom",
];

/// One file's content and filesystem semantics.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceFile {
    /// SHA-256 of file contents, including an internal symlink's target contents.
    pub sha256: String,
    /// Whether any Unix executable bit is set.
    pub executable: bool,
    /// Exact relative link target, when the selected entry is a symlink.
    pub link: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Boundary {
    // Platform-owned public runtime trees and standard devices, not selections.
    pub read_only_paths: Vec<PathBuf>,
    pub snapshot_mounts: Vec<PathBuf>,
    pub snapshot_files: BTreeMap<PathBuf, SourceFile>,
    pub snapshot_directories: Vec<PathBuf>,
    pub environment: BTreeMap<String, String>,
    pub sandbox: PathBuf,
}

impl Boundary {
    /// Count private inodes, including both snapshot bytes and mount placeholders.
    /// The separate work tmpfs and attached public trees do not consume this quota.
    pub fn validate_snapshot_inodes(
        &self,
        cwd: &std::path::Path,
        work: &std::path::Path,
    ) -> std::io::Result<usize> {
        use std::{collections::BTreeSet, io, path::Path};
        let mut paths = BTreeSet::new();
        let mut reserve = |path: PathBuf| -> io::Result<()> {
            for parent in path.ancestors() {
                paths.insert(parent.to_owned());
                if paths.len() > 131_072 {
                    return Err(io::Error::other(
                        "snapshot structural inode budget exceeded",
                    ));
                }
            }
            Ok(())
        };
        let relative = |path: &Path| -> io::Result<PathBuf> {
            let path = path.strip_prefix("/").map_err(io::Error::other)?;
            if path
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                return Err(io::Error::other("non-normal snapshot path"));
            }
            Ok(path.into())
        };
        for path in [
            ".qualification-inputs",
            ".qualification-boundary.json",
            "proc",
        ] {
            reserve(path.into())?;
        }
        for path in self.snapshot_files.keys().chain(&self.snapshot_directories) {
            reserve(Path::new(".qualification-inputs").join(relative(path)?))?;
        }
        for path in self
            .snapshot_mounts
            .iter()
            .chain(&self.read_only_paths)
            .map(PathBuf::as_path)
            .chain([cwd, work])
        {
            reserve(relative(path)?)?;
        }
        Ok(paths.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn final_review_snapshot_admission_counts_structural_inodes_and_placeholders() {
        let mut policy = Boundary {
            read_only_paths: vec!["/usr/bin".into()],
            snapshot_mounts: Vec::new(),
            snapshot_files: BTreeMap::new(),
            snapshot_directories: Vec::new(),
            environment: BTreeMap::new(),
            sandbox: "/sandbox".into(),
        };
        let base = policy
            .validate_snapshot_inodes(Path::new("/cwd"), Path::new("/work"))
            .unwrap();
        let file = SourceFile {
            sha256: "a".repeat(64),
            executable: false,
            link: None,
        };
        for i in 0..65_531 {
            let path = PathBuf::from(format!("/file-{i}"));
            policy.snapshot_mounts.push(path.clone());
            policy.snapshot_files.insert(path, file.clone());
        }
        assert_eq!(
            base + 2 * 65_531,
            policy
                .validate_snapshot_inodes(Path::new("/cwd"), Path::new("/work"))
                .unwrap()
        );
        // Every selected leaf needs its private bytes and its visible mount target.
        for i in 65_531..65_536 {
            let path = PathBuf::from(format!("/file-{i}"));
            policy.snapshot_mounts.push(path.clone());
            policy.snapshot_files.insert(path, file.clone());
        }
        assert!(
            policy
                .validate_snapshot_inodes(Path::new("/cwd"), Path::new("/work"))
                .is_err()
        );
    }

    #[test]
    fn final_review_snapshot_admission_deduplicates_shared_parents() {
        let mut policy = Boundary {
            read_only_paths: vec!["/usr/bin".into()],
            snapshot_mounts: vec!["/src".into()],
            snapshot_files: BTreeMap::new(),
            snapshot_directories: vec!["/src/nested".into()],
            environment: BTreeMap::new(),
            sandbox: "/sandbox".into(),
        };
        let base = policy
            .validate_snapshot_inodes(Path::new("/src"), Path::new("/work"))
            .unwrap();
        for name in ["a", "b"] {
            policy.snapshot_files.insert(
                format!("/src/nested/{name}").into(),
                SourceFile {
                    sha256: "a".repeat(64),
                    executable: false,
                    link: None,
                },
            );
        }
        assert_eq!(
            base + 2,
            policy
                .validate_snapshot_inodes(Path::new("/src"), Path::new("/work"))
                .unwrap()
        );
        policy.snapshot_mounts.push("/src/nested".into());
        assert_eq!(
            base + 3,
            policy
                .validate_snapshot_inodes(Path::new("/src/nested"), Path::new("/work"))
                .unwrap()
        );
    }
}
