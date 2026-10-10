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
const DEPTH_LIMIT: usize = 64;

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
    /// Empty structural directories retired after exact descendant removals.
    /// Every root requires acknowledged descendant identities in `delete`.
    #[serde(default)]
    pub prune: Vec<String>,
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
    /// Empty directory transition roots durably retired by this exchange.
    #[serde(default)]
    pub pruned: Vec<String>,
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

fn bounded_encoding(batch: &Batch) -> io::Result<()> {
    struct EncodedSize(usize);
    impl Write for EncodedSize {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            if self.0 > PAYLOAD_LIMIT {
                return Err(invalid("encoded evidence payload exceeds its bound"));
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(EncodedSize(0), batch).map_err(|error| invalid(error.to_string()))
}

fn relative(path: &Path, allowed: &[PathBuf]) -> io::Result<()> {
    if path.as_os_str().is_empty()
        || path.components().count() > DEPTH_LIMIT
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
    if allowed.len() > INVENTORY_LIMIT || known.len() > INVENTORY_LIMIT {
        return Err(invalid(
            "evidence root/acknowledgement inventory exceeds its bound",
        ));
    }
    collect_with(root, allowed, known, &mut |_| {})
}

fn collect_with(
    root: &Path,
    allowed: &[PathBuf],
    known: &BTreeMap<String, String>,
    directory_observed: &mut impl FnMut(&Path),
) -> io::Result<Batch> {
    for path in allowed {
        relative(path, std::slice::from_ref(path))?;
    }
    fn walk(
        parent: &File,
        path: &Path,
        known: &BTreeMap<String, String>,
        batch: &mut Batch,
        size: &mut usize,
        entries: &mut usize,
        directory_observed: &mut impl FnMut(&Path),
    ) -> io::Result<()> {
        if path.components().count() > DEPTH_LIMIT {
            return Err(invalid("evidence traversal depth exceeds its bound"));
        }
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
                    *entries += 1;
                    if *entries > INVENTORY_LIMIT {
                        return Err(invalid(
                            "aggregate evidence directory inventory exceeds its bound",
                        ));
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
                        entries,
                        directory_observed,
                    )?;
                }
            }
        } else {
            let prefix = format!("{name}/");
            if known
                .range(prefix.clone()..)
                .next()
                .is_some_and(|(known, _)| known.starts_with(&prefix))
            {
                // Retire acknowledged descendants and their empty structural
                // containers before sending this directory-to-file replacement.
                batch.prune.push(name.clone());
                batch.deferred.push(name);
                return Ok(());
            }
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
    let mut entries = 0;
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
        entries += 1;
        if entries > INVENTORY_LIMIT {
            return Err(invalid("aggregate evidence inventory exceeds its bound"));
        }
        relative(path, allowed)?;
        if let Some(parent) = receiver_parent(&root, path, false)? {
            walk(
                &parent,
                path,
                known,
                &mut batch,
                &mut size,
                &mut entries,
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
        let transition_descendant = batch
            .prune
            .iter()
            .any(|name| path.starts_with(name) && path != Path::new(name));
        let deleted = if transition_descendant {
            true
        } else {
            match receiver_parent(&root, path, false)? {
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
                                    return Err(invalid(
                                        "evidence identity became a project checkout",
                                    ));
                                }
                                true
                            }
                            _ => false,
                        },
                        Err(rustix::io::Errno::NOENT) => true,
                        Err(error) => return Err(error.into()),
                    }
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
    for name in &batch.prune {
        let path = Path::new(name);
        if known.iter().any(|(name, sha)| {
            Path::new(name).starts_with(path)
                && !batch
                    .delete
                    .iter()
                    .any(|item| &item.path == name && &item.sha256 == sha)
        }) {
            return Err(invalid(
                "directory transition descendant removals exceed one-exchange budget",
            ));
        }
    }
    for name in batch
        .omitted
        .iter()
        .chain(&batch.deferred)
        .chain(&batch.prune)
    {
        size = size.saturating_add(name.len().saturating_add(1024));
    }
    if size > PAYLOAD_LIMIT {
        return Err(invalid("evidence inventory exceeds its payload bound"));
    }
    bounded_encoding(&batch)?;
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
///
/// Shared references cannot apply simultaneous mutations under one retained lease:
///
/// ```compile_fail
/// # #[cfg(all(unix, feature = "orchestration"))] {
/// use canix_toolbelt::orchestration::evidence::{Batch, Mirror};
/// use std::{path::{Path, PathBuf}, sync::Arc};
/// let mirror = Arc::new(Mirror::acquire(Path::new("private-mirror")).unwrap());
/// let first = Arc::clone(&mirror);
/// let second = Arc::clone(&mirror);
/// std::thread::scope(|scope| {
///     scope.spawn(move || first.apply(&[PathBuf::from("evidence")], &Batch::default()));
///     scope.spawn(move || second.apply(&[PathBuf::from("evidence")], &Batch::default()));
/// });
/// # }
/// ```
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

    /// Apply one exchange under an exclusive mutable borrow of the retained
    /// lease. Sharing a guard cannot admit concurrent exchanges under one flock.
    pub fn apply(&mut self, allowed: &[PathBuf], batch: &Batch) -> io::Result<Receipt> {
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
#[derive(Debug)]
struct CheckoutTraversal;

impl std::fmt::Display for CheckoutTraversal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("evidence path traverses a project checkout")
    }
}

impl std::error::Error for CheckoutTraversal {}

fn blocked_traversal(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|cause| cause.is::<CheckoutTraversal>())
        || matches!(error.raw_os_error(), Some(code) if code == rustix::io::Errno::NOTDIR.raw_os_error() || code == rustix::io::Errno::LOOP.raw_os_error())
}

fn receiver_parent(root: &File, path: &Path, create: bool) -> io::Result<Option<File>> {
    if create {
        receiver_parent_with(root, path, true, &mut File::sync_all)
    } else {
        // Source collection and comparison reads establish no durable receipt.
        receiver_parent_with(root, path, false, &mut |_| Ok(()))
    }
}

fn receiver_parent_with(
    root: &File,
    path: &Path,
    create: bool,
    sync: &mut impl FnMut(&File) -> io::Result<()>,
) -> io::Result<Option<File>> {
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
            Err(rustix::io::Errno::NOENT) => {
                // A replay may observe a directory entry removed by a previous
                // failed exchange. Its absence needs durable parent evidence.
                sync(&parent)?;
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        // Also sync existing entries: a previous failed exchange may have
        // created this ancestor without durably acknowledging its link.
        sync(&parent)?;
        parent = File::from(fd);
        match rustix::fs::statat(&parent, ".git", AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    CheckoutTraversal,
                ));
            }
            Err(rustix::io::Errno::NOENT) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Some(parent))
}

#[derive(PartialEq)]
enum Receiver {
    Missing,
    File(String),
    Foreign,
}

impl Receiver {
    fn matches(&self, digest: &str) -> bool {
        matches!(self,Self::File(sha) if sha == digest)
    }
    fn expected(&self, digest: Option<&String>) -> bool {
        match digest {
            Some(sha) => self.matches(sha),
            None => matches!(self, Self::Missing),
        }
    }
}

fn receiver_hash(root: &File, path: &Path) -> io::Result<Receiver> {
    let Some(parent) = receiver_parent(root, path, false)? else {
        return Ok(Receiver::Missing);
    };
    let name = path
        .file_name()
        .ok_or_else(|| invalid("artifact has no filename"))?;
    receiver_hash_at(&parent, name)
}

fn receiver_hash_at(parent: &File, name: &std::ffi::OsStr) -> io::Result<Receiver> {
    let meta = match rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(meta) => meta,
        Err(rustix::io::Errno::NOENT) => return Ok(Receiver::Missing),
        Err(error) => return Err(error.into()),
    };
    if FileType::from_raw_mode(meta.st_mode) != FileType::RegularFile
        || meta.st_size < 0
        || meta.st_size as u64 > FILE_LIMIT
    {
        return Ok(Receiver::Foreign);
    }
    let fd = match rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(Receiver::Missing),
        Err(rustix::io::Errno::LOOP) => return Ok(Receiver::Foreign),
        Err(error) => return Err(error.into()),
    };
    let file = File::from(fd);
    if !file.metadata()?.is_file() || file.metadata()?.len() > FILE_LIMIT {
        return Ok(Receiver::Foreign);
    }
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Ok(Receiver::Foreign);
    }
    Ok(Receiver::File(hash(&bytes)))
}

#[cfg(test)]
fn write(parent: &File, name: &std::ffi::OsStr, bytes: &[u8], sha: &str) -> io::Result<()> {
    write_with(parent, name, bytes, sha, &mut File::sync_all)
}

fn write_with(
    parent: &File,
    name: &std::ffi::OsStr,
    bytes: &[u8],
    sha: &str,
    sync: &mut impl FnMut(&File) -> io::Result<()>,
) -> io::Result<()> {
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
        sync(parent)
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
    apply_locked_with(root, allowed, batch, &mut File::sync_all)
}

fn apply_locked_with(
    root: &File,
    allowed: &[PathBuf],
    batch: &Batch,
    sync: &mut impl FnMut(&File) -> io::Result<()>,
) -> io::Result<Receipt> {
    bounded_encoding(batch)?;
    for path in allowed {
        relative(path, std::slice::from_ref(path))?;
    }
    if batch.files.len().saturating_add(batch.delete.len()) > FILE_COUNT_LIMIT
        || batch.verify.len() > INVENTORY_LIMIT
        || batch.prune.len() > INVENTORY_LIMIT
        || batch.omitted.len().saturating_add(batch.deferred.len()) > INVENTORY_LIMIT
    {
        return Err(invalid("evidence file count exceeds its bound"));
    }
    let mut size = batch
        .omitted
        .iter()
        .chain(&batch.deferred)
        .chain(&batch.prune)
        .fold(0usize, |size, name| {
            size.saturating_add(name.len().saturating_add(1024))
        });
    if size > PAYLOAD_LIMIT {
        return Err(invalid("evidence inventory exceeds its payload bound"));
    }
    let mut paths = std::collections::BTreeSet::new();
    let mut blocked_retirements = std::collections::BTreeSet::new();
    let mut retirement_roots = std::collections::BTreeSet::new();
    for name in &batch.prune {
        let path = Path::new(name);
        relative(path, allowed)?;
        retirement_roots.insert(path.to_path_buf());
        // Inspect only pinned ancestors, never contents within a checkout or
        // foreign non-directory root. Such roots block their transition alone.
        match receiver_parent(root, &path.join("retirement-probe"), false) {
            Ok(_) => {}
            Err(error) if blocked_traversal(&error) => {
                blocked_retirements.insert(path.to_path_buf());
            }
            Err(error) => return Err(error),
        }
    }
    // Validate every deletion ancestor before any mutation. A checkout may be
    // nested below a structural root rather than at the root itself.
    for item in &batch.delete {
        let path = Path::new(&item.path);
        relative(path, allowed)?;
        if let Some(transition) = path
            .ancestors()
            .skip(1)
            .find(|parent| retirement_roots.contains(*parent))
        {
            match receiver_parent(root, path, false) {
                Ok(_) => {}
                Err(error) if blocked_traversal(&error) => {
                    blocked_retirements.insert(transition.to_path_buf());
                }
                Err(error) => return Err(error),
            }
        }
    }
    let retirement_blocked = |path: &Path| {
        path.ancestors()
            .any(|ancestor| blocked_retirements.contains(ancestor))
    };
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
        if !retirement_blocked(Path::new(&item.path)) {
            receiver_hash(root, Path::new(&item.path))?;
        }
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
    let mut pruning = std::collections::BTreeSet::new();
    for name in &batch.prune {
        let path = Path::new(name);
        relative(path, allowed)?;
        if !pruning.insert(path.to_path_buf())
            || !batch.delete.iter().any(|item| {
                Path::new(&item.path).starts_with(path) && Path::new(&item.path) != path
            })
            || batch
                .files
                .iter()
                .map(|item| &item.path)
                .chain(batch.verify.iter().map(|item| &item.path))
                .any(|name| Path::new(name).starts_with(path) || path.starts_with(name))
        {
            return Err(invalid(
                "directory retirement lacks scoped descendant removal identities",
            ));
        }
    }
    for path in &pruning {
        if path
            .ancestors()
            .skip(1)
            .any(|parent| pruning.contains(parent))
        {
            return Err(invalid("overlapping directory retirement roots"));
        }
    }
    let mut receipt = Receipt::default();
    for item in &batch.verify {
        if receiver_hash(root, Path::new(&item.path))?.matches(&item.sha256) {
            let parent = receiver_parent_with(root, Path::new(&item.path), false, sync)?
                .ok_or_else(|| invalid("receiver verification parent moved"))?;
            sync(&parent)?;
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
        if !current.matches(&item.sha256) && !current.expected(item.expected.as_ref()) {
            receipt.conflicts.push(item.path.clone());
            continue;
        }
        let parent = receiver_parent_with(root, path, true, sync)?.expect("created parent");
        let name = path.file_name().expect("validated name");
        let pinned = receiver_hash_at(&parent, name)?;
        if !pinned.matches(&item.sha256) && !pinned.expected(item.expected.as_ref()) {
            receipt.conflicts.push(item.path.clone());
            continue;
        }
        if !pinned.matches(&item.sha256) {
            write_with(&parent, name, &item.bytes, &item.sha256, sync)?;
        } else {
            // Byte equality after a failed rename fsync is not durability.
            sync(&parent)?;
        }
        receipt
            .accepted
            .insert(item.path.clone(), item.sha256.clone());
    }
    for item in &batch.delete {
        let path = Path::new(&item.path);
        if retirement_blocked(path) {
            receipt.conflicts.push(item.path.clone());
            continue;
        }
        let current = receiver_hash(root, path)?;
        if current != Receiver::Missing && !current.matches(&item.sha256) {
            receipt.conflicts.push(item.path.clone());
            continue;
        }
        if let Some(parent) = receiver_parent_with(root, path, false, sync)? {
            let name = path.file_name().expect("validated name");
            let pinned = receiver_hash_at(&parent, name)?;
            if pinned != Receiver::Missing && !pinned.matches(&item.sha256) {
                receipt.conflicts.push(item.path.clone());
                continue;
            }
            if pinned != Receiver::Missing {
                rustix::fs::unlinkat(&parent, name, AtFlags::empty())?;
            }
            sync(&parent)?;
        } else if current != Receiver::Missing {
            return Err(invalid("receiver deletion parent moved"));
        }
        receipt
            .removed
            .insert(item.path.clone(), item.sha256.clone());
    }
    for path in pruning {
        let descendants: Vec<_> = batch
            .delete
            .iter()
            .filter(|item| Path::new(&item.path).starts_with(&path))
            .collect();
        let mut complete = !blocked_retirements.contains(&path)
            && descendants
                .iter()
                .all(|item| receipt.removed.get(&item.path) == Some(&item.sha256));
        let mut directories = std::collections::BTreeSet::new();
        for item in &descendants {
            for directory in Path::new(&item.path)
                .ancestors()
                .skip(1)
                .take_while(|parent| parent.starts_with(&path))
            {
                directories.insert(directory.to_path_buf());
            }
        }
        // Descendants precede parents; only empty containers are retired.
        for directory in directories.into_iter().rev() {
            if !complete {
                break;
            }
            complete = prune_empty(root, &directory, sync)?;
        }
        if complete {
            receipt.pruned.push(path.to_string_lossy().into_owned());
        } else {
            receipt.conflicts.push(path.to_string_lossy().into_owned());
            // Keep all transition identities until the structural retirement
            // succeeds. A later replay can safely acknowledge absent leaves.
            for item in descendants {
                receipt.removed.remove(&item.path);
            }
        }
    }
    Ok(receipt)
}

fn prune_empty(
    root: &File,
    path: &Path,
    sync: &mut impl FnMut(&File) -> io::Result<()>,
) -> io::Result<bool> {
    let Some(parent) = receiver_parent_with(root, path, false, sync)? else {
        return Ok(true);
    };
    let name = path.file_name().expect("validated directory filename");
    match rustix::fs::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => {
            sync(&parent)?;
            return Ok(true);
        }
        Ok(meta) if FileType::from_raw_mode(meta.st_mode) == FileType::Directory => {}
        Ok(_) => return Ok(false),
        Err(error) => return Err(error.into()),
    }
    let directory = File::from(rustix::fs::openat(
        &parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    if checkout(&directory)? {
        return Ok(false);
    }
    match rustix::fs::unlinkat(&parent, name, AtFlags::REMOVEDIR) {
        Ok(()) => {
            sync(&parent)?;
            Ok(true)
        }
        Err(rustix::io::Errno::NOTEMPTY) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn directory_retirement_fsync_failure_cannot_be_acknowledged_by_an_absent_replay() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let root = open_directory(target.path()).unwrap();
        let allowed = vec![PathBuf::from("evidence")];
        fs::create_dir_all(source.path().join("evidence/a/nested")).unwrap();
        fs::write(source.path().join("evidence/a/nested/b"), b"old").unwrap();
        let first = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
        let known = apply_locked(&root, &allowed, &first).unwrap().accepted;
        fs::remove_dir_all(source.path().join("evidence/a")).unwrap();
        fs::write(source.path().join("evidence/a"), b"new").unwrap();
        let transition = collect(source.path(), &allowed, &known).unwrap();
        let mut fail_after_retirement = |parent: &File| {
            if !target.path().join("evidence/a").exists() {
                Err(io::Error::other("injected post-rmdir fsync failure"))
            } else {
                parent.sync_all()
            }
        };
        assert!(
            apply_locked_with(&root, &allowed, &transition, &mut fail_after_retirement).is_err()
        );
        assert!(!target.path().join("evidence/a").exists());
        assert!(
            apply_locked_with(&root, &allowed, &transition, &mut fail_after_retirement).is_err()
        );
        let receipt = apply_locked(&root, &allowed, &transition).unwrap();
        assert_eq!(receipt.removed, known);
        assert_eq!(receipt.pruned, vec!["evidence/a"]);
    }

    #[test]
    fn post_rename_sync_failure_cannot_be_acknowledged_by_an_idempotent_replay() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let root = open_directory(target.path()).unwrap();
        let allowed = vec![PathBuf::from("handoff")];
        fs::write(source.path().join("handoff"), b"durable evidence").unwrap();
        let batch = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
        let mut sync_attempts = 0;
        let mut failing_sync = |_: &File| {
            sync_attempts += 1;
            Err(io::Error::other(
                "injected post-rename directory fsync failure",
            ))
        };
        assert!(apply_locked_with(&root, &allowed, &batch, &mut failing_sync).is_err());
        assert_eq!(
            fs::read(target.path().join("handoff")).unwrap(),
            b"durable evidence"
        );
        assert!(
            apply_locked_with(&root, &allowed, &batch, &mut failing_sync).is_err(),
            "digest equality acknowledged a still-unsynced rename"
        );
        assert_eq!(sync_attempts, 2);
        let receipt = apply_locked(&root, &allowed, &batch).unwrap();
        assert_eq!(receipt.accepted["handoff"], batch.files[0].sha256);
    }

    #[test]
    fn post_unlink_sync_failure_cannot_be_acknowledged_by_an_absent_file_replay() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let root = open_directory(target.path()).unwrap();
        let allowed = vec![PathBuf::from("handoff")];
        fs::write(source.path().join("handoff"), b"durable evidence").unwrap();
        let initial = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
        let known = apply_locked(&root, &allowed, &initial).unwrap().accepted;
        fs::remove_file(source.path().join("handoff")).unwrap();
        let deletion = collect(source.path(), &allowed, &known).unwrap();
        let mut sync_attempts = 0;
        let mut failing_sync = |_: &File| {
            sync_attempts += 1;
            Err(io::Error::other(
                "injected post-unlink directory fsync failure",
            ))
        };
        assert!(apply_locked_with(&root, &allowed, &deletion, &mut failing_sync).is_err());
        assert!(!target.path().join("handoff").exists());
        assert!(
            apply_locked_with(&root, &allowed, &deletion, &mut failing_sync).is_err(),
            "absence acknowledged a still-unsynced unlink"
        );
        assert_eq!(sync_attempts, 2);
        assert_eq!(
            apply_locked(&root, &allowed, &deletion).unwrap().removed,
            known
        );
    }
    #[test]
    fn ancestor_sync_failure_prevents_further_receiver_directory_creation() {
        let target = tempfile::tempdir().unwrap();
        let root = open_directory(target.path()).unwrap();
        let mut sync_attempts = 0;
        let result = receiver_parent_with(
            &root,
            Path::new("evidence/nested/receipt"),
            true,
            &mut |_| {
                sync_attempts += 1;
                Err(io::Error::other("injected directory fsync failure"))
            },
        );
        assert!(result.is_err(), "ancestor durability failure was ignored");
        assert_eq!(sync_attempts, 1);
        assert!(!target.path().join("evidence/nested").exists());
        // A retry must sync even an ancestor left by the failed attempt.
        let mut retry_syncs = 0;
        receiver_parent_with(
            &root,
            Path::new("evidence/nested/receipt"),
            true,
            &mut |parent| {
                retry_syncs += 1;
                parent.sync_all()
            },
        )
        .unwrap();
        assert_eq!(
            retry_syncs, 2,
            "retry skipped an existing but unacknowledged ancestor"
        );
    }
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
