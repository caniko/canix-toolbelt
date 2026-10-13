//! Source-bound qualification without checkout, toolchain or fleet defaults.
//!
//! Inventories and receipts are evidence, not signatures or test-coverage claims.
//! Callers select all source roots, inputs, required gates and execution policy.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

mod receipt;
pub use receipt::*;
pub mod graph;
mod legacy;
pub use legacy::*;
mod seal;
pub use seal::*;
#[cfg(feature = "qualification-cli")]
pub mod cli;

/// Sorted portable artifact paths and SHA-256 hashes (also the Python index format).
pub type ArtifactIndex = BTreeMap<String, String>;

/// Explicit source selection. No build-directory exclusions are inferred.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventorySpec {
    /// Root containing every selected source and symlink target.
    pub root: PathBuf,
    /// Root-relative files or directories, recursively selected.
    pub paths: Vec<PathBuf>,
}

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

/// Versioned source inventory. Link text and executable bits bind source identity.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Inventory {
    /// Wire format version; currently 1.
    pub schema_version: u32,
    /// Sorted, root-relative entries.
    pub files: BTreeMap<String, SourceFile>,
}

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Hash a file incrementally, without loading an archive or log into memory.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Stable pretty JSON bytes, with two-space indentation and a final newline.
pub fn json_bytes(value: &impl Serialize) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(invalid_json)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(crate) fn invalid_json(error: serde_json::Error) -> io::Error {
    invalid(error.to_string())
}

/// Load a typed JSON document without interpreting any embedded commands.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    serde_json::from_reader(File::open(path)?).map_err(invalid_json)
}

pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&json_bytes(value)?)?;
    file.sync_all()
}

/// SHA-256 identity of the canonical serialization of a typed value.
pub fn fingerprint(value: &impl Serialize) -> io::Result<String> {
    Ok(format!("{:x}", Sha256::digest(json_bytes(value)?)))
}

pub(crate) fn relative(path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err(invalid(format!(
            "expected a root-relative path: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn contained(root: &Path, path: &Path) -> io::Result<PathBuf> {
    relative(path)?;
    let resolved = root.join(path).canonicalize()?;
    if !resolved.starts_with(root) {
        return Err(invalid(format!("path escapes root: {}", path.display())));
    }
    Ok(resolved)
}

pub(crate) fn path_text(path: &Path) -> io::Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("non-UTF-8 paths cannot be represented in a portable inventory"))
}

fn collect(root: &Path, path: &Path, files: &mut BTreeMap<String, SourceFile>) -> io::Result<()> {
    let resolved = contained(root, path)?;
    let metadata = fs::symlink_metadata(root.join(path))?;
    if metadata.is_dir() {
        for entry in fs::read_dir(root.join(path))? {
            let child = entry?.path();
            if child.file_name().is_some_and(|name| name == ".git") {
                return Err(invalid(
                    "select source paths explicitly instead of recursively including .git",
                ));
            }
            collect(
                root,
                child
                    .strip_prefix(root)
                    .map_err(|e| invalid(e.to_string()))?,
                files,
            )?;
        }
        return Ok(());
    }
    if !resolved.is_file() {
        return Err(invalid(format!(
            "not a regular source file: {}",
            path.display()
        )));
    }
    let link = if metadata.file_type().is_symlink() {
        let target = fs::read_link(root.join(path))?;
        if target.is_absolute() {
            return Err(invalid("absolute symlinks are not portable source"));
        }
        Some(path_text(&target)?)
    } else {
        None
    };
    #[cfg(unix)]
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(&resolved)?.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let executable = false;
    let key = path_text(
        root.join(path)
            .strip_prefix(root)
            .map_err(|e| invalid(e.to_string()))?,
    )?;
    files.insert(
        key.trim_start_matches("./").to_owned(),
        SourceFile {
            sha256: sha256_file(&resolved)?,
            executable,
            link,
        },
    );
    Ok(())
}

/// Capture exact selected bytes; escaping symlinks and special files fail closed.
pub fn inventory(spec: &InventorySpec) -> io::Result<Inventory> {
    if spec.paths.is_empty() {
        return Err(invalid("source selection is empty"));
    }
    let root = spec.root.canonicalize()?;
    let mut files = BTreeMap::new();
    for path in &spec.paths {
        collect(&root, path, &mut files)?;
    }
    if files.is_empty() {
        return Err(invalid("source inventory is empty"));
    }
    Ok(Inventory {
        schema_version: 1,
        files,
    })
}

/// Select Git-tracked and unignored untracked files using NUL-delimited names.
/// Gitlinks require an explicit independent inventory, rather than hidden recursion.
pub fn git_inventory_spec(root: &Path, git: &Path) -> io::Result<InventorySpec> {
    let output = std::process::Command::new(git)
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "-co", "--exclude-standard"])
        .output()?;
    if !output.status.success() {
        return Err(invalid(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let mut paths = Vec::new();
    for name in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let path = PathBuf::from(std::str::from_utf8(name).map_err(|e| invalid(e.to_string()))?);
        if root.join(&path).is_dir() {
            return Err(invalid(format!(
                "gitlink requires a separate source selection: {}",
                path.display()
            )));
        }
        paths.push(path);
    }
    paths.sort();
    paths.dedup();
    Ok(InventorySpec {
        root: root.into(),
        paths,
    })
}

/// Verify every selected artifact's bytes. Extra files do not establish acceptance.
pub fn verify_index(root: &Path, index: &ArtifactIndex) -> io::Result<usize> {
    if index.is_empty() {
        return Err(invalid("artifact index is empty"));
    }
    let root = root.canonicalize()?;
    for (path, expected) in index {
        if sha256_file(&contained(&root, Path::new(path))?)? != *expected {
            return Err(invalid(format!("artifact hash mismatch: {path}")));
        }
    }
    Ok(index.len())
}
