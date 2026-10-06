//! Toolbelt-owned, offline Git preparation. A prepared snapshot is not a qualified
//! worker and cannot authorize dispatch. Forge observation and dispatch remain
//! Toolbelt obligations; this boundary never fetches, runs agents or publishes.

#![deny(unsafe_code)]
#![cfg(target_os = "linux")]

pub mod admission;
mod budget;
mod completion;
pub mod execution;
mod files;
mod git;
pub mod offline;
pub mod peer;
mod platform;
mod supervisor;

pub use supervisor::helper_main;

use anyhow::{Context, Result, ensure};
use nix::fcntl::{Flock, FlockArg};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

/// Preparation coordinates supplied by the trusted controller after fresh forge
/// authorization. This data type is deliberately not an authorization token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub request_url: String,
    pub request_id: String,
    /// SHA-256 of the complete authorized request, including comparison/policy.
    pub authorized_request_sha256: String,
    /// SHA-256 of the intended concrete execution policy, not proof it is qualified.
    pub execution_policy_sha256: String,
    pub base: String,
    pub head: String,
}

impl Binding {
    /// The durable namespace does not change when a comparison/policy changes.
    pub fn key(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(b"canix-roborev-preparation-request-v1\0");
        for value in [&self.request_url, &self.request_id] {
            hash.update((value.len() as u64).to_le_bytes());
            hash.update(value.as_bytes());
        }
        format!("{:x}", hash.finalize())
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.request_url.starts_with("https://")
                && self.request_url.len() <= 2048
                && !self.request_url.chars().any(char::is_control)
                && !self.request_id.trim().is_empty()
                && self.request_id.len() <= 512
                && !self.request_id.chars().any(char::is_control),
            "invalid preparation request identity"
        );
        ensure!(
            is_hex(&self.authorized_request_sha256, 64)
                && is_hex(&self.execution_policy_sha256, 64)
                && [40, 64].contains(&self.head.len())
                && is_hex(&self.head, self.head.len())
                && is_hex(&self.base, self.head.len()),
            "preparation requires exact full commits and request/policy digests"
        );
        Ok(())
    }
}

/// Explicit immutable tools; no PATH lookup or namespace fallback is allowed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    pub git: PathBuf,
    pub bubblewrap: PathBuf,
    /// Immutable executable implementing the private preparation-helper protocol.
    pub preparer: PathBuf,
}

/// Bounds for preparation only. These are persisted with the snapshot and do
/// not define or attest an agent's cgroup/egress/resource policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub input_bytes: u64,
    pub snapshot_bytes: u64,
    pub entries: usize,
    pub wall_seconds: u64,
    pub memory_bytes: u64,
    pub cpu_seconds: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            input_bytes: 256 * 1024 * 1024,
            snapshot_bytes: 512 * 1024 * 1024,
            entries: 50_000,
            wall_seconds: 90,
            memory_bytes: 1024 * 1024 * 1024,
            cpu_seconds: 30,
        }
    }
}

impl Limits {
    fn validate(&self) -> Result<()> {
        ensure!(
            (1..=1024 * 1024 * 1024).contains(&self.input_bytes)
                && (self.input_bytes..=2 * 1024 * 1024 * 1024).contains(&self.snapshot_bytes)
                && (1..=100_000).contains(&self.entries)
                && (1..=300).contains(&self.wall_seconds)
                && (64 * 1024 * 1024..=4 * 1024 * 1024 * 1024).contains(&self.memory_bytes)
                && (1..=120).contains(&self.cpu_seconds),
            "preparation limits exceed the bounded local contract"
        );
        Ok(())
    }
}

/// Content-bound preparation output. Read-only modes and this digest are not
/// lifetime isolation against the same UID. No production verifier consumes this
/// as a worker-qualification receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prepared {
    pub checkout: PathBuf,
    pub snapshot_sha256: String,
    pub binding: Binding,
    journal: PathBuf,
}

impl Prepared {
    pub fn journal_path(&self) -> PathBuf {
        self.journal.clone()
    }
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: u32,
    anchor: Anchor,
    binding: Binding,
    tools: Tools,
    limits: Limits,
    snapshot_sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Anchor {
    entry: PathBuf,
    directory_device: u64,
    directory_inode: u64,
    lock_device: u64,
    lock_inode: u64,
    lock_identity: String,
}

fn lock_identity(mut file: &File, fresh: bool) -> Result<String> {
    if fresh {
        let mut identity = [0u8; 32];
        getrandom::fill(&mut identity).map_err(|error| {
            anyhow::anyhow!("cannot establish original lock anchor identity: {error}")
        })?;
        file.write_all(&identity)?;
        file.sync_all()?;
    }
    ensure!(
        file.metadata()?.len() == 32,
        "invalid original lock anchor identity"
    );
    let mut identity = [0u8; 32];
    file.read_exact_at(&mut identity, 0)?;
    Ok(format!("{:x}", Sha256::digest(identity)))
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn save(path: &Path, journal: &Journal) -> Result<()> {
    let parent = path
        .parent()
        .context("preparation journal needs a parent")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&serde_json::to_vec(journal)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

/// Copy local raw objects into a sanitized, request-exclusive detached checkout.
/// An interrupted or missing preparation is retained for explicit diagnosis, not
/// silently replayed under another checkout/ledger identity. The caller must hold
/// authorization; this operation has no forge/provider/daemon effects.
pub fn prepare_local(
    objects: &Path,
    state: &Path,
    binding: &Binding,
    tools: &Tools,
    limits: &Limits,
) -> Result<Prepared> {
    binding.validate()?;
    limits.validate()?;
    let tools = git::validated_tools(tools)?;
    supervisor::prepare(objects, state, binding, &tools, limits)
}

fn prepare_in_process(
    objects: &Path,
    state: &Path,
    binding: &Binding,
    tools: &Tools,
    limits: &Limits,
) -> Result<Prepared> {
    binding.validate()?;
    limits.validate()?;
    let budget = budget::Budget::new(limits)?;
    let tools = git::validated_tools(tools)?;
    files::private_directory(state)?;
    let entry = state.join(binding.key());
    files::private_directory(&entry)?;
    let lock_path = entry.join("request.lock");
    let path = entry.join("preparation.json");
    ensure!(
        lock_path.try_exists()? || !path.try_exists()?,
        "preparation journal exists without its original lock anchor"
    );
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK | nix::libc::O_CLOEXEC);
    let (lock, fresh) = match options.clone().create_new(true).open(&lock_path) {
        Ok(lock) => {
            lock.sync_all()?;
            File::open(&entry)?.sync_all()?;
            (lock, true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            (options.open(&lock_path)?, false)
        }
        Err(error) => return Err(error.into()),
    };
    files::private_file(&lock)?;
    let _lock = Flock::lock(lock, FlockArg::LockExclusiveNonblock)
        .map_err(|(_, error)| anyhow::anyhow!("another preparer owns this request: {error}"))?;
    let directory = entry.metadata()?;
    let lock_metadata = _lock.metadata()?;
    let lock_identity = lock_identity(&_lock, fresh)?;
    let checkout = entry.join("checkout");
    let mut journal = Journal {
        schema_version: 2,
        anchor: Anchor {
            entry: entry.clone(),
            directory_device: directory.dev(),
            directory_inode: directory.ino(),
            lock_device: lock_metadata.dev(),
            lock_inode: lock_metadata.ino(),
            lock_identity,
        },
        binding: binding.clone(),
        tools: tools.clone(),
        limits: limits.clone(),
        snapshot_sha256: None,
    };
    let old = match files::read_private(&path) {
        Ok(bytes) => {
            Some(serde_json::from_slice::<Journal>(&bytes).context("invalid preparation journal")?)
        }
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            None
        }
        Err(error) => return Err(error),
    };
    let digest = if let Some(old) = old {
        ensure!(
            !fresh && journal.anchor == old.anchor,
            "preparation journal differs from its original lock anchor or state path"
        );
        journal.snapshot_sha256 = old.snapshot_sha256.clone();
        ensure!(
            journal == old,
            "request was reused with changed preparation identity"
        );
        let digest = old
            .snapshot_sha256
            .context("preparation is incomplete; retain the same request for diagnosis")?;
        ensure!(
            files::snapshot(&checkout, limits, &budget, false)? == digest,
            "prepared snapshot changed"
        );
        digest
    } else {
        ensure!(
            fresh,
            "preparation journal missing behind a claimed request; no rematerialization permitted"
        );
        ensure!(!checkout.exists(), "unclaimed checkout already exists");
        save(&path, &journal)?;
        files::private_directory(&checkout)?;
        let format = if binding.head.len() == 40 {
            "--object-format=sha1"
        } else {
            "--object-format=sha256"
        };
        git::run(
            &tools,
            &checkout,
            limits,
            &budget,
            &["init", "--quiet", "--template=", format],
        )?;
        let copied = files::copy_objects(
            objects,
            &checkout.join(".git/objects"),
            limits,
            &budget,
            binding.head.len(),
        )?;
        git::run(
            &tools,
            &checkout,
            limits,
            &budget,
            &[
                "fsck",
                "--strict",
                "--no-reflogs",
                &binding.base,
                &binding.head,
            ],
        )?;
        for commit in [&binding.base, &binding.head] {
            let kind = git::run(
                &tools,
                &checkout,
                limits,
                &budget,
                &["cat-file", "-t", commit],
            )?;
            ensure!(
                kind.trim() == "commit",
                "preparation requires a raw commit object for both coordinates"
            );
        }
        git::check_expansion(&tools, &checkout, limits, &budget, &binding.head, &copied)?;
        files::neutralize_attributes(&checkout)?;
        git::run(
            &tools,
            &checkout,
            limits,
            &budget,
            &["checkout", "--quiet", "--force", "--detach", &binding.head],
        )?;
        let actual = git::run(
            &tools,
            &checkout,
            limits,
            &budget,
            &["rev-parse", "--verify", "HEAD"],
        )?;
        ensure!(
            actual.trim() == binding.head,
            "materialized head differs from the frozen comparison"
        );
        let digest = files::snapshot(&checkout, limits, &budget, true)?;
        budget.check()?;
        journal.snapshot_sha256 = Some(digest.clone());
        save(&path, &journal)?;
        digest
    };
    budget.check()?;
    Ok(Prepared {
        checkout,
        snapshot_sha256: digest,
        binding: binding.clone(),
        journal: path,
    })
}
