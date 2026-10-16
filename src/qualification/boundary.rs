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
