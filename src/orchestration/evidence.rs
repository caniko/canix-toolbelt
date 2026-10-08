//! Bounded, byte-exact evidence replication. This module never resumes agents.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

const FILE_LIMIT: u64 = 8 * 1024 * 1024;
const PAYLOAD_LIMIT: usize = 48 * 1024 * 1024;

/// One source-bound artifact with the receiver's previously acknowledged identity.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    /// Campaign-relative path within explicitly allowed owner roots.
    pub path: String,
    /// Digest of the exact raw bytes.
    pub sha256: String,
    /// Previous receiver digest; unknown foreign changes cause a conflict.
    pub expected: Option<String>,
    /// Exact bytes, including binary attachments.
    pub bytes: Vec<u8>,
}

/// A bounded transfer and host-local/deferred inventory.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    /// Changed artifacts eligible for replication.
    pub files: Vec<Artifact>,
    /// Symlinks, project checkouts and oversized artifacts left on their owner host.
    pub omitted: Vec<String>,
    /// Eligible changes left for the next bounded exchange.
    pub deferred: Vec<String>,
}

/// Native acknowledgement; only accepted identities may advance the exchange journal.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Receipt {
    /// Accepted paths and exact digests, including idempotent replays.
    pub accepted: BTreeMap<String, String>,
    /// Paths whose receiver bytes changed independently; both copies remain intact.
    pub conflicts: Vec<String>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn relative(path: &Path, allowed: &[PathBuf]) -> io::Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || path
            .components()
            .any(|c| matches!(c.as_os_str().to_str(), Some(".git" | "worktrees")))
        || !allowed.iter().any(|root| path.starts_with(root))
    {
        return Err(invalid(
            "artifact path is outside declared owner evidence roots",
        ));
    }
    Ok(())
}

fn ancestors(root: &Path, relative: &Path) -> io::Result<()> {
    if !fs::symlink_metadata(root)?.is_dir() {
        return Err(invalid("evidence root must be a real directory"));
    }
    let mut path = root.to_path_buf();
    for c in relative.components() {
        path.push(c);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(invalid("evidence path traverses a symlink"));
            }
            Ok(meta) if meta.is_dir() && path.join(".git").try_exists()? => {
                return Err(invalid("evidence path traverses a project checkout"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn read(path: &Path) -> io::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > FILE_LIMIT {
        return Err(invalid("artifact is not a bounded regular file"));
    }
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(invalid("artifact grew beyond its bound"));
    }
    Ok(bytes)
}

/// Collect declared authoritative paths, excluding checkouts and symlinks at any depth.
pub fn collect(
    root: &Path,
    allowed: &[PathBuf],
    known: &BTreeMap<String, String>,
) -> io::Result<Batch> {
    fn walk(
        root: &Path,
        path: &Path,
        known: &BTreeMap<String, String>,
        batch: &mut Batch,
        size: &mut usize,
    ) -> io::Result<()> {
        let full = root.join(path);
        let meta = match fs::symlink_metadata(&full) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        let name = path
            .to_str()
            .ok_or_else(|| invalid("artifact path must be UTF-8"))?
            .to_owned();
        if meta.file_type().is_symlink()
            || path
                .components()
                .any(|c| matches!(c.as_os_str().to_str(), Some(".git" | "worktrees")))
            || (meta.is_dir() && full.join(".git").try_exists()?)
            || (!meta.is_dir() && (!meta.is_file() || meta.len() > FILE_LIMIT))
        {
            batch.omitted.push(name);
        } else if meta.is_dir() {
            let mut children = fs::read_dir(full)?.collect::<Result<Vec<_>, _>>()?;
            children.sort_by_key(|entry| entry.file_name());
            for child in children {
                walk(root, &path.join(child.file_name()), known, batch, size)?;
            }
        } else {
            let bytes = read(&full)?;
            let sha = hash(&bytes);
            if known.get(&name) == Some(&sha) {
                return Ok(());
            }
            let encoded = bytes
                .len()
                .saturating_mul(4)
                .saturating_add(name.len() + 1024);
            if size.saturating_add(encoded) > PAYLOAD_LIMIT || batch.files.len() >= 10_000 {
                batch.deferred.push(name);
            } else {
                *size += encoded;
                batch.files.push(Artifact {
                    expected: known.get(&name).cloned(),
                    path: name,
                    sha256: sha,
                    bytes,
                });
            }
        }
        if batch.omitted.len() + batch.deferred.len() > 20_000 {
            return Err(invalid("evidence inventory exceeds its bound"));
        }
        Ok(())
    }
    let mut batch = Batch::default();
    let mut size = 0;
    for path in allowed {
        relative(path, allowed)?;
        if let Some(parent) = path.parent() {
            ancestors(root, parent)?;
        }
        walk(root, path, known, &mut batch, &mut size)?;
    }
    batch.files.sort_by(|a, b| a.path.cmp(&b.path));
    batch.files.dedup_by(|a, b| a.path == b.path);
    Ok(batch)
}

fn write(path: &Path, bytes: &[u8], sha: &str) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("artifact has no parent"))?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let temp = parent.join(format!(".native-evidence-{}-{sha}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        File::open(parent)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

/// Verify scope/digests before writing and retain independently edited receiver files.
/// Call under the shared exchange lease, against a private replication directory.
pub fn apply(root: &Path, allowed: &[PathBuf], batch: &Batch) -> io::Result<Receipt> {
    let mut size = 0usize;
    let mut paths = std::collections::BTreeSet::new();
    for item in &batch.files {
        relative(Path::new(&item.path), allowed)?;
        ancestors(root, Path::new(&item.path))?;
        size = size.saturating_add(
            item.bytes
                .len()
                .saturating_mul(4)
                .saturating_add(item.path.len() + 1024),
        );
        if !paths.insert(&item.path)
            || item.bytes.len() as u64 > FILE_LIMIT
            || size > PAYLOAD_LIMIT
            || hash(&item.bytes) != item.sha256
        {
            return Err(invalid("invalid or oversized evidence payload"));
        }
    }
    let mut receipt = Receipt::default();
    for item in &batch.files {
        let path = root.join(&item.path);
        let current = match read(&path) {
            Ok(bytes) => Some(hash(&bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if current.as_ref() != Some(&item.sha256) && current.is_some() && current != item.expected {
            receipt.conflicts.push(item.path.clone());
            continue;
        }
        if current.as_ref() != Some(&item.sha256) {
            write(&path, &item.bytes, &item.sha256)?;
        }
        receipt
            .accepted
            .insert(item.path.clone(), item.sha256.clone());
    }
    Ok(receipt)
}
