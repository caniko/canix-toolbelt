//! Bounded, byte-exact evidence replication. This module never resumes agents.
use fs2::FileExt;
use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read, Write},
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const FILE_LIMIT: u64 = 8 * 1024 * 1024;
const PAYLOAD_LIMIT: usize = 48 * 1024 * 1024;
const FILE_COUNT_LIMIT: usize = 10_000;
const INVENTORY_LIMIT: usize = 20_000;

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

/// An acknowledged path whose raw-byte identity is verified or deleted.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    /// Campaign-relative path within explicitly allowed owner roots.
    pub path: String,
    /// Previously acknowledged digest required at the receiver.
    pub sha256: String,
}

/// A bounded transfer and host-local/deferred inventory.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    /// Changed artifacts eligible for replication.
    pub files: Vec<Artifact>,
    /// Unchanged source identities, checked without retransmitting raw bytes.
    #[serde(default)]
    pub verify: Vec<Identity>,
    /// Authoritative deletions, conditional on acknowledged receiver bytes.
    #[serde(default)]
    pub delete: Vec<Identity>,
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
    /// Removed paths and prior digests. Remove these exact bindings from `known`
    /// only after retaining this receipt; deletion replays are idempotent.
    #[serde(default)]
    pub removed: BTreeMap<String, String>,
    /// Paths whose receiver bytes changed independently; both copies remain intact.
    pub conflicts: Vec<String>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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

fn read_at(parent: &File, name: &std::ffi::OsStr) -> io::Result<Vec<u8>> {
    let file = File::from(rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
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
    collect_with(root, allowed, known, &mut |_| {})
}

fn collect_with(
    root: &Path,
    allowed: &[PathBuf],
    known: &BTreeMap<String, String>,
    directory_observed: &mut impl FnMut(&Path),
) -> io::Result<Batch> {
    fn walk(
        parent: &File,
        path: &Path,
        known: &BTreeMap<String, String>,
        batch: &mut Batch,
        size: &mut usize,
        directory_observed: &mut impl FnMut(&Path),
    ) -> io::Result<()> {
        let leaf = path
            .file_name()
            .ok_or_else(|| invalid("artifact has no filename"))?;
        let meta = match rustix::fs::statat(parent, leaf, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(m) => m,
            Err(rustix::io::Errno::NOENT) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let name = path
            .to_str()
            .ok_or_else(|| invalid("artifact path must be UTF-8"))?
            .to_owned();
        let kind = FileType::from_raw_mode(meta.st_mode);
        if path
            .components()
            .any(|c| matches!(c.as_os_str().to_str(), Some(".git" | "worktrees")))
            || !matches!(kind, FileType::Directory | FileType::RegularFile)
            || (kind == FileType::RegularFile
                && (meta.st_size < 0 || meta.st_size as u64 > FILE_LIMIT))
        {
            batch.omitted.push(name);
        } else if kind == FileType::Directory {
            let directory = File::from(rustix::fs::openat(
                parent,
                leaf,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?);
            if checkout(&directory)? {
                batch.omitted.push(name);
            } else if known.contains_key(&name) {
                // First remove the previously acknowledged file identity. The
                // retained removal receipt releases descendants next exchange.
                batch.deferred.push(name);
            } else {
                directory_observed(path);
                let mut children = Vec::new();
                for entry in rustix::fs::Dir::read_from(&directory)? {
                    let entry = entry?;
                    let bytes = entry.file_name().to_bytes();
                    if matches!(bytes, b"." | b"..") {
                        continue;
                    }
                    if children.len() >= INVENTORY_LIMIT {
                        return Err(invalid("evidence directory inventory exceeds its bound"));
                    }
                    children.push(std::ffi::OsStr::from_bytes(bytes).to_owned());
                }
                children.sort();
                for child in children {
                    walk(
                        &directory,
                        &path.join(child),
                        known,
                        batch,
                        size,
                        directory_observed,
                    )?;
                }
            }
        } else {
            let bytes = read_at(parent, leaf)?;
            let sha = hash(&bytes);
            if known.get(&name) == Some(&sha) {
                let encoded = name.len().saturating_add(1024);
                if batch.verify.len() >= INVENTORY_LIMIT
                    || size.saturating_add(encoded) > PAYLOAD_LIMIT
                {
                    return Err(invalid(
                        "acknowledged evidence verification exceeds its bound",
                    ));
                }
                *size += encoded;
                batch.verify.push(Identity {
                    path: name,
                    sha256: sha,
                });
                return Ok(());
            }
            let encoded = bytes
                .len()
                .saturating_mul(4)
                .saturating_add(name.len() + 1024);
            if size.saturating_add(encoded) > PAYLOAD_LIMIT || batch.files.len() >= FILE_COUNT_LIMIT
            {
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
        if batch.omitted.len() + batch.deferred.len() > INVENTORY_LIMIT {
            return Err(invalid("evidence inventory exceeds its bound"));
        }
        Ok(())
    }
    let mut batch = Batch::default();
    let root = open_directory(root)?;
    let mut size = 0;
    let mut roots = allowed.to_vec();
    roots.sort();
    let mut unique = Vec::<PathBuf>::new();
    for path in roots {
        relative(&path, allowed)?;
        if !unique.iter().any(|root| path.starts_with(root)) {
            unique.push(path);
        }
    }
    for path in &unique {
        relative(path, allowed)?;
        if let Some(parent) = receiver_parent(&root, path, false)? {
            walk(
                &parent,
                path,
                known,
                &mut batch,
                &mut size,
                directory_observed,
            )?;
        }
    }
    for (name, sha) in known {
        let path = Path::new(name);
        if !unique.iter().any(|root| path.starts_with(root)) {
            continue;
        }
        relative(path, &unique)?;
        // Resolve deletion discovery through pinned ancestors too. A replaced
        // symlink or checkout never supplies authority to delete receiver bytes.
        let deleted = match receiver_parent(&root, path, false)? {
            None => true,
            Some(parent) => {
                let leaf = path.file_name().expect("validated filename");
                match rustix::fs::statat(&parent, leaf, AtFlags::SYMLINK_NOFOLLOW) {
                    Ok(meta) => match FileType::from_raw_mode(meta.st_mode) {
                        FileType::Symlink => {
                            return Err(invalid("evidence identity became a symlink"));
                        }
                        FileType::Directory => {
                            let directory = File::from(rustix::fs::openat(
                                &parent,
                                leaf,
                                OFlags::RDONLY
                                    | OFlags::DIRECTORY
                                    | OFlags::NOFOLLOW
                                    | OFlags::CLOEXEC,
                                Mode::empty(),
                            )?);
                            if checkout(&directory)? {
                                return Err(invalid("evidence identity became a project checkout"));
                            }
                            true
                        }
                        _ => false,
                    },
                    Err(rustix::io::Errno::NOENT) => true,
                    Err(error) => return Err(error.into()),
                }
            }
        };
        if deleted {
            if !valid_digest(sha) {
                return Err(invalid("malformed acknowledged evidence identity"));
            }
            let encoded = name.len().saturating_add(1024);
            if batch.files.len() + batch.delete.len() >= FILE_COUNT_LIMIT
                || size.saturating_add(encoded) > PAYLOAD_LIMIT
            {
                batch.deferred.push(name.clone());
                if batch.omitted.len() + batch.deferred.len() > INVENTORY_LIMIT {
                    return Err(invalid("evidence inventory exceeds its bound"));
                }
            } else {
                size += encoded;
                batch.delete.push(Identity {
                    path: name.clone(),
                    sha256: sha.clone(),
                });
            }
        }
    }
    batch.files.sort_by(|a, b| a.path.cmp(&b.path));
    batch.files.dedup_by(|a, b| a.path == b.path);
    for name in batch.omitted.iter().chain(&batch.deferred) {
        size = size.saturating_add(name.len().saturating_add(1024));
    }
    if size > PAYLOAD_LIMIT {
        return Err(invalid("evidence inventory exceeds its payload bound"));
    }
    Ok(batch)
}

fn open_directory(path: &Path) -> io::Result<File> {
    Ok(File::from(rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?))
}

fn checkout(directory: &File) -> io::Result<bool> {
    match rustix::fs::statat(directory, ".git", AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Ok(true),
        Err(rustix::io::Errno::NOENT) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Exclusive lease for a private receiver root. Every authorized mirror writer
/// must hold this lease, including local edits and legacy adapters. The pinned
/// root inode is the shared kernel lock anchor; never replace it during exchange.
/// This serializes cooperative writers, not processes bypassing the contract.
pub struct Mirror {
    root: File,
}

impl Mirror {
    /// Pin and lease a private mirror without waiting. No receiver mutation is
    /// allowed before this succeeds. Root directories must be mode 0700.
    pub fn acquire(root: &Path) -> io::Result<Self> {
        let root = open_directory(root)?;
        if root.metadata()?.permissions().mode() & 0o777 != 0o700 {
            return Err(invalid("evidence mirror root must be private (mode 0700)"));
        }
        root.try_lock_exclusive()?;
        Ok(Self { root })
    }

    /// Apply one exchange while retaining exclusive mirror ownership.
    pub fn apply(&self, allowed: &[PathBuf], batch: &Batch) -> io::Result<Receipt> {
        apply_locked(&self.root, allowed, batch)
    }
}

impl Drop for Mirror {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.root);
    }
}

// Every receiver operation resolves relative to pinned, no-follow directory
// handles. A concurrent ancestor symlink replacement cannot redirect it.
fn receiver_parent(root: &File, path: &Path, create: bool) -> io::Result<Option<File>> {
    let mut parent = root.try_clone()?;
    for component in path
        .parent()
        .ok_or_else(|| invalid("artifact has no parent"))?
        .components()
    {
        let Component::Normal(name) = component else {
            return Err(invalid("invalid receiver ancestor"));
        };
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let fd = match rustix::fs::openat(&parent, name, flags, Mode::empty()) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) if create => {
                match rustix::fs::mkdirat(&parent, name, Mode::from_bits_truncate(0o700)) {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                    Err(error) => return Err(error.into()),
                }
                rustix::fs::openat(&parent, name, flags, Mode::empty())?
            }
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        parent = File::from(fd);
        match rustix::fs::statat(&parent, ".git", AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => return Err(invalid("evidence path traverses a project checkout")),
            Err(rustix::io::Errno::NOENT) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Some(parent))
}

fn receiver_hash(root: &File, path: &Path) -> io::Result<Option<String>> {
    let Some(parent) = receiver_parent(root, path, false)? else {
        return Ok(None);
    };
    let name = path
        .file_name()
        .ok_or_else(|| invalid("artifact has no filename"))?;
    receiver_hash_at(&parent, name)
}

fn receiver_hash_at(parent: &File, name: &std::ffi::OsStr) -> io::Result<Option<String>> {
    let fd = match rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let file = File::from(fd);
    if !file.metadata()?.is_file() || file.metadata()?.len() > FILE_LIMIT {
        return Err(invalid("artifact is not a bounded regular file"));
    }
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(invalid("artifact grew beyond its bound"));
    }
    Ok(Some(hash(&bytes)))
}

fn write(parent: &File, name: &std::ffi::OsStr, bytes: &[u8], sha: &str) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut staging = None;
    for _ in 0..32 {
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let temp = format!(".native-evidence-{}-{serial}-{sha}", std::process::id());
        match rustix::fs::openat(
            parent,
            &temp,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_bits_truncate(0o600),
        ) {
            Ok(fd) => {
                staging = Some((temp, File::from(fd)));
                break;
            }
            Err(rustix::io::Errno::EXIST) => {}
            Err(error) => return Err(error.into()),
        }
    }
    let (temp, mut file) =
        staging.ok_or_else(|| invalid("evidence staging collisions exceed their bound"))?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        rustix::fs::renameat(parent, &temp, parent, name)?;
        parent.sync_all()
    })();
    if result.is_err() {
        let _ = rustix::fs::unlinkat(parent, &temp, AtFlags::empty());
    }
    result
}

/// Acquire the private mirror's shared lease, then verify/apply one exchange.
/// For a longer authorized mutation, retain `Mirror` and call `Mirror::apply`.
pub fn apply(root: &Path, allowed: &[PathBuf], batch: &Batch) -> io::Result<Receipt> {
    Mirror::acquire(root)?.apply(allowed, batch)
}

fn apply_locked(root: &File, allowed: &[PathBuf], batch: &Batch) -> io::Result<Receipt> {
    if batch.files.len().saturating_add(batch.delete.len()) > FILE_COUNT_LIMIT
        || batch.verify.len() > INVENTORY_LIMIT
        || batch.omitted.len().saturating_add(batch.deferred.len()) > INVENTORY_LIMIT
    {
        return Err(invalid("evidence file count exceeds its bound"));
    }
    let mut size = batch
        .omitted
        .iter()
        .chain(&batch.deferred)
        .fold(0usize, |size, name| {
            size.saturating_add(name.len().saturating_add(1024))
        });
    if size > PAYLOAD_LIMIT {
        return Err(invalid("evidence inventory exceeds its payload bound"));
    }
    let mut paths = std::collections::BTreeSet::new();
    for item in &batch.files {
        relative(Path::new(&item.path), allowed)?;
        receiver_hash(root, Path::new(&item.path))?;
        size = size.saturating_add(
            item.bytes
                .len()
                .saturating_mul(4)
                .saturating_add(item.path.len() + 1024),
        );
        if !paths.insert(PathBuf::from(&item.path))
            || item.bytes.len() as u64 > FILE_LIMIT
            || size > PAYLOAD_LIMIT
            || hash(&item.bytes) != item.sha256
            || item.expected.as_ref().is_some_and(|sha| !valid_digest(sha))
        {
            return Err(invalid("invalid or oversized evidence payload"));
        }
    }
    for item in batch.verify.iter().chain(&batch.delete) {
        relative(Path::new(&item.path), allowed)?;
        receiver_hash(root, Path::new(&item.path))?;
        size = size.saturating_add(item.path.len().saturating_add(1024));
        if !paths.insert(PathBuf::from(&item.path))
            || !valid_digest(&item.sha256)
            || size > PAYLOAD_LIMIT
        {
            return Err(invalid("invalid or oversized evidence identity inventory"));
        }
    }
    for path in &paths {
        if path
            .ancestors()
            .skip(1)
            .any(|parent| paths.contains(parent))
        {
            return Err(invalid(
                "evidence payload contains a file and its descendant",
            ));
        }
    }
    let mut receipt = Receipt::default();
    for item in &batch.verify {
        if receiver_hash(root, Path::new(&item.path))?.as_ref() == Some(&item.sha256) {
            receipt
                .accepted
                .insert(item.path.clone(), item.sha256.clone());
        } else {
            receipt.conflicts.push(item.path.clone());
        }
    }
    for item in &batch.files {
        let path = Path::new(&item.path);
        let current = receiver_hash(root, path)?;
        if current.as_ref() != Some(&item.sha256) && current != item.expected {
            receipt.conflicts.push(item.path.clone());
            continue;
        }
        if current.as_ref() != Some(&item.sha256) {
            let parent = receiver_parent(root, path, true)?.expect("created parent");
            let name = path.file_name().expect("validated name");
            let pinned = receiver_hash_at(&parent, name)?;
            if pinned.as_ref() != Some(&item.sha256) && pinned != item.expected {
                receipt.conflicts.push(item.path.clone());
                continue;
            }
            if pinned.as_ref() != Some(&item.sha256) {
                write(&parent, name, &item.bytes, &item.sha256)?;
            }
        }
        receipt
            .accepted
            .insert(item.path.clone(), item.sha256.clone());
    }
    for item in &batch.delete {
        let path = Path::new(&item.path);
        let current = receiver_hash(root, path)?;
        if current.is_some() && current.as_ref() != Some(&item.sha256) {
            receipt.conflicts.push(item.path.clone());
            continue;
        }
        if current.is_some() {
            let parent = receiver_parent(root, path, false)?
                .ok_or_else(|| invalid("receiver deletion parent moved"))?;
            let name = path.file_name().expect("validated name");
            let pinned = receiver_hash_at(&parent, name)?;
            if pinned.is_some() && pinned.as_ref() != Some(&item.sha256) {
                receipt.conflicts.push(item.path.clone());
                continue;
            }
            if pinned.is_some() {
                rustix::fs::unlinkat(&parent, name, AtFlags::empty())?;
            }
            parent.sync_all()?;
        }
        receipt
            .removed
            .insert(item.path.clone(), item.sha256.clone());
    }
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn source_ancestor_replacement_never_collects_outside_bytes() {
        let source = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(source.path().join("evidence")).unwrap();
        fs::write(source.path().join("evidence/receipt"), b"owned evidence").unwrap();
        fs::write(outside.path().join("receipt"), b"OUTSIDE SENTINEL").unwrap();
        let mut changed = false;
        let result = collect_with(
            source.path(),
            &[PathBuf::from("evidence")],
            &BTreeMap::new(),
            &mut |path| {
                if path == Path::new("evidence") && !changed {
                    changed = true;
                    fs::rename(
                        source.path().join("evidence"),
                        source.path().join("retained"),
                    )
                    .unwrap();
                    std::os::unix::fs::symlink(outside.path(), source.path().join("evidence"))
                        .unwrap();
                }
            },
        );
        assert!(changed, "did not exercise the ancestor replacement");
        if let Ok(batch) = result {
            assert!(
                batch
                    .files
                    .iter()
                    .all(|item| item.bytes != b"OUTSIDE SENTINEL")
            );
            assert_eq!(batch.files.len(), 1);
            assert_eq!(batch.files[0].bytes, b"owned evidence");
        }
    }

    #[test]
    fn pinned_receiver_parent_cannot_follow_a_concurrent_ancestor_symlink() {
        let target = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(target.path().join("evidence")).unwrap();
        let root = File::from(
            rustix::fs::open(
                target.path(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap(),
        );
        let parent = receiver_parent(&root, Path::new("evidence/receipt"), false)
            .unwrap()
            .unwrap();
        fs::rename(
            target.path().join("evidence"),
            target.path().join("retained"),
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path(), target.path().join("evidence")).unwrap();
        let bytes = b"exact scoped evidence";
        write(
            &parent,
            std::ffi::OsStr::new("receipt"),
            bytes,
            &hash(bytes),
        )
        .unwrap();
        assert!(!outside.path().join("receipt").exists());
        assert_eq!(
            fs::read(target.path().join("retained/receipt")).unwrap(),
            bytes
        );
        assert!(receiver_parent(&root, Path::new("evidence/receipt"), true).is_err());
    }
}
