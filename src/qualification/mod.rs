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

mod boundary;
mod receipt;
pub use receipt::*;
mod coverage;
pub use coverage::*;
mod execution;
pub use execution::ExecutionReport;
pub mod graph;
mod legacy;
pub use legacy::*;
mod seal;
pub use seal::*;
#[cfg(feature = "qualification-cli")]
pub mod cli;

/// Sorted portable artifact paths and SHA-256 hashes (also the Python index format).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct ArtifactIndex(
    #[serde(deserialize_with = "unique_artifact_map")] BTreeMap<String, String>,
);

fn unique_artifact_map<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    unique_map_named(deserializer, "artifact index")
}

pub(crate) fn unique_map<'de, D, V>(deserializer: D) -> Result<BTreeMap<String, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    V: Deserialize<'de>,
{
    unique_map_named(deserializer, "qualification map")
}

fn unique_map_named<'de, D, V>(
    deserializer: D,
    name: &'static str,
) -> Result<BTreeMap<String, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct Unique<V>(&'static str, std::marker::PhantomData<V>);
    impl<'de, V: Deserialize<'de>> serde::de::Visitor<'de> for Unique<V> {
        type Value = BTreeMap<String, V>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(formatter, "a {} with unique keys", self.0)
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut values = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, V>()? {
                if values.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom(format!(
                        "duplicate {} key",
                        self.0
                    )));
                }
            }
            Ok(values)
        }
    }
    deserializer.deserialize_map(Unique(name, std::marker::PhantomData))
}

impl ArtifactIndex {
    /// Construct an empty, sorted index.
    pub fn new() -> Self {
        Self::default()
    }
}
impl std::ops::Deref for ArtifactIndex {
    type Target = BTreeMap<String, String>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ArtifactIndex {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl<const N: usize> From<[(String, String); N]> for ArtifactIndex {
    fn from(values: [(String, String); N]) -> Self {
        Self(values.into())
    }
}
impl From<BTreeMap<String, String>> for ArtifactIndex {
    fn from(values: BTreeMap<String, String>) -> Self {
        Self(values)
    }
}
impl FromIterator<(String, String)> for ArtifactIndex {
    fn from_iter<T: IntoIterator<Item = (String, String)>>(values: T) -> Self {
        Self(values.into_iter().collect())
    }
}
impl IntoIterator for ArtifactIndex {
    type Item = (String, String);
    type IntoIter = std::collections::btree_map::IntoIter<String, String>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
impl<'a> IntoIterator for &'a ArtifactIndex {
    type Item = (&'a String, &'a String);
    type IntoIter = std::collections::btree_map::Iter<'a, String, String>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

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
    #[serde(deserialize_with = "unique_map")]
    pub files: BTreeMap<String, SourceFile>,
}

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub(crate) fn regular_file(path: &Path) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Validate the opened descriptor, not a racy path stat. In particular,
        // opening a FIFO must never wait for a writer before we can reject it.
        options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
    }
    #[cfg(not(unix))]
    if !fs::metadata(path)?.is_file() {
        return Err(invalid("not a regular file"));
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid(format!("not a regular file: {}", path.display())));
    }
    Ok(file)
}

/// Hash an opened regular file incrementally. Devices and FIFOs are rejected.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    sha256_file_checked(path, &mut || Ok(()))
}

pub(crate) fn sha256_file_checked(
    path: &Path,
    check: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<String> {
    check()?;
    let file = regular_file(path)?;
    let before = file.metadata()?;
    let bound = before
        .len()
        .checked_add(1)
        .ok_or_else(|| invalid("file length cannot be bounded"))?;
    let mut input = (&file).take(bound);
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        check()?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    let after = file.metadata()?;
    // One extra byte detects pseudo-files that report zero length but expose
    // content, as well as growth that otherwise hides behind the read bound.
    if input.limit() != 1
        || before.len() != after.len()
        || before.modified()? != after.modified()?
    {
        return Err(invalid("file changed while hashing"));
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

/// Load at most 64 MiB of typed JSON from one opened regular-file descriptor.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    read_bound_json(path, None)
}

// Parse exactly the bytes whose independently selected digest was checked.
// Reopening the path after hashing would allow another attempt to substitute
// its passing document. Bound the read even if a concurrent writer grows it.
pub(crate) fn read_bound_json<T: serde::de::DeserializeOwned>(
    path: &Path,
    expected: Option<&str>,
) -> io::Result<T> {
    let file = regular_file(path)?;
    read_bound_json_file(file, expected)
}

fn read_bound_json_file<T: serde::de::DeserializeOwned>(
    file: File,
    expected: Option<&str>,
) -> io::Result<T> {
    let before = file.metadata()?;
    if before.len() > 64 * 1024 * 1024 {
        return Err(invalid("evidence JSON exceeds 64 MiB"));
    }
    let mut bytes = Vec::new();
    (&file).take(before.len() + 1).read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    if bytes.len() as u64 != before.len()
        || after.len() != before.len()
        || before.modified()? != after.modified()?
    {
        return Err(invalid("evidence JSON changed while reading"));
    }
    if expected.is_some_and(|hash| format!("{:x}", Sha256::digest(&bytes)) != hash) {
        return Err(invalid("selected evidence JSON digest differs"));
    }
    serde_json::from_slice(&bytes).map_err(invalid_json)
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
        || path_text(path)?.contains(['\\', ':'])
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

pub(crate) fn artifact_key(name: &str) -> io::Result<()> {
    if name.contains(['\\', ':']) {
        return Err(invalid("artifact keys must use slash-only portable paths"));
    }
    relative(Path::new(name))
}

// Retained evidence must survive relocation. Disallow symlink hops, including
// in-root absolute links and directory links, rather than following host paths.
pub(crate) fn evidence_path(root: &Path, path: &Path) -> io::Result<PathBuf> {
    relative(path)?;
    let mut selected = root.to_path_buf();
    for part in path.components() {
        if let Component::Normal(name) = part {
            selected.push(name);
            if fs::symlink_metadata(&selected)?.file_type().is_symlink() {
                return Err(invalid("symlinks are not portable evidence artifacts"));
            }
        }
    }
    contained(root, path)
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
    path.components()
        .map(|part| match part {
            Component::Normal(name) => {
                let text = name.to_str().ok_or_else(|| {
                    invalid("non-UTF-8 paths cannot be represented in a portable inventory")
                })?;
                // Unix permits drive-relative names and backslashes that have
                // different path semantics on Windows, including in link text.
                if text.contains(['\\', ':']) {
                    return Err(invalid("backslash or colon in a portable path component"));
                }
                Ok(text.to_owned())
            }
            Component::CurDir => Ok(".".into()),
            Component::ParentDir => Ok("..".into()),
            _ => Err(invalid("absolute paths are not portable inventory paths")),
        })
        .collect::<io::Result<Vec<_>>>()
        .map(|parts| parts.join("/"))
}

fn collect(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<String, SourceFile>,
    check: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    check()?;
    let resolved = contained(root, path)?;
    let metadata = fs::symlink_metadata(root.join(path))?;
    if metadata.is_dir() {
        let mut entries = fs::read_dir(root.join(path))?.peekable();
        if entries.peek().is_none() {
            return Err(invalid(
                "empty directories cannot be represented by source inventory v1",
            ));
        }
        for entry in entries {
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
                check,
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
            sha256: sha256_file_checked(&resolved, check)?,
            executable,
            link,
        },
    );
    Ok(())
}

/// Capture exact selected bytes; escaping symlinks and special files fail closed.
pub fn inventory(spec: &InventorySpec) -> io::Result<Inventory> {
    inventory_checked(spec, &mut || Ok(()))
}

pub(crate) fn inventory_checked(
    spec: &InventorySpec,
    check: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<Inventory> {
    if spec.paths.is_empty() {
        return Err(invalid("source selection is empty"));
    }
    let root = spec.root.canonicalize()?;
    let mut files = BTreeMap::new();
    for path in &spec.paths {
        collect(&root, path, &mut files, check)?;
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
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_NAMESPACE")
        .env_remove("GIT_PREFIX")
        .env_remove("GIT_CEILING_DIRECTORIES")
        .env_remove("GIT_DISCOVERY_ACROSS_FILESYSTEM")
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
        artifact_key(path)?;
        if sha256_file(&evidence_path(&root, Path::new(path))?)? != *expected {
            return Err(invalid(format!("artifact hash mismatch: {path}")));
        }
    }
    Ok(index.len())
}

#[cfg(test)]
mod evidence_tests {
    use super::*;

    #[test]
    fn artifact_indexes_and_embedded_receipt_requirements_reject_duplicates() {
        let bytes = r#"{"gate":"failed-receipt-hash","gate":"passing-receipt-hash"}"#;
        assert!(
            serde_json::from_str::<ArtifactIndex>(bytes)
                .unwrap_err()
                .to_string()
                .contains("duplicate artifact index key")
        );
        let composition =
            r#"{"schemaVersion":1,"sources":{"source":"first","source":"second"},"inputs":{}}"#;
        assert!(serde_json::from_str::<Composition>(composition).is_err());
        let index: ArtifactIndex = [("a".into(), "digest".into())].into();
        assert_eq!(serde_json::to_string(&index).unwrap(), r#"{"a":"digest"}"#);
    }

    #[test]
    fn selected_json_cannot_be_replaced_by_a_different_attempt_after_open() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("receipt.json");
        fs::write(&path, br#"{"status":"failed"}"#).unwrap();
        let expected = sha256_file(&path).unwrap();
        let selected = regular_file(&path).unwrap();
        fs::rename(&path, root.path().join("selected.json")).unwrap();
        fs::write(&path, br#"{"status":"passed"}"#).unwrap();
        let observed: serde_json::Value = read_bound_json_file(selected, Some(&expected)).unwrap();
        assert_eq!(observed["status"], "failed");
        assert!(read_bound_json::<serde_json::Value>(&path, Some(&expected)).is_err());
    }
}
