//! Controller-side, one-shot backend reservation. This is durable accounting,
//! not worker authentication or a production confinement verifier. The controller
//! must protect this store from daemon/backend writes and authenticate the job,
//! input custody and execution policy before reserving. No agents are launched.

use super::{Anchor, Binding, files, is_hex, lock_identity};
use anyhow::{Context, Result, ensure};
use nix::fcntl::{Flock, FlockArg};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
};

/// Fixed request, persisted job and intended backend execution. The referenced
/// manifests must include all concrete identities/content described by the worker
/// contract. Hash syntax checks here do not verify those manifests or authorize
/// their contents. Observed backend/session identities arise after reservation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBinding {
    pub request: Binding,
    pub controller_id: String,
    /// Manifest of exact daemon data-directory/database custody and generation.
    pub daemon_identity_sha256: String,
    pub job_id: u64,
    pub job_uuid: String,
    /// Preallocated Canix identity, distinct from later OpenCode session IDs.
    pub execution_id: String,
    /// Complete checkout, prompt/references, coverage and effective-policy inputs.
    pub input_manifest_sha256: String,
    /// Intended executable/configuration/backend namespace and execution policy.
    pub backend_manifest_sha256: String,
}

impl ExecutionBinding {
    pub(super) fn validate(&self) -> Result<()> {
        self.request.validate()?;
        ensure!(
            self.job_id > 0
                && [
                    &self.controller_id,
                    &self.daemon_identity_sha256,
                    &self.execution_id,
                    &self.input_manifest_sha256,
                    &self.backend_manifest_sha256
                ]
                .iter()
                .all(|value| is_hex(value, 64))
                && self.job_uuid.len() == 36
                && self.job_uuid.split('-').map(str::len).eq([8, 4, 4, 4, 12])
                && self
                    .job_uuid
                    .split('-')
                    .all(|value| is_hex(value, value.len())),
            "execution requires exact controller, daemon/job and input/backend identities"
        );
        Ok(())
    }
}

/// UNKNOWN is committed before returning any reservation. It means execution may
/// have happened, even if the caller crashed before starting a backend. It cannot
/// transition back to Ready; later completion/re-delivery needs a separate receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Ready,
    Unknown,
}

/// Persist this original registration in the controller's request ledger before
/// dispatch. Loading/reserving never recreates missing state. Registration belongs
/// only to the controller's pre-dispatch new-request path; loss of its wider ledger
/// must not be repaired by registering a replacement under fresh identities.
#[cfg_attr(
    not(any(feature = "roborev-execution-tests", feature = "roborev-worker-tests")),
    doc = r#"
Production controllers obtain reservations through original `Admission` custody.
Standalone registration, loading and reservation are native-fixture APIs only.

```compile_fail
use canix_toolbelt_roborev_worker::execution::ExecutionFence;
let _ = ExecutionFence::register;
```

```compile_fail
use canix_toolbelt_roborev_worker::execution::ExecutionFence;
let _ = ExecutionFence::load;
```

```compile_fail
use canix_toolbelt_roborev_worker::execution::ExecutionFence;
let _ = ExecutionFence::reserve;
```
"#
)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionFence {
    anchor: Anchor,
    binding: ExecutionBinding,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: u32,
    fence: ExecutionFence,
    state: ExecutionState,
}

/// Non-cloneable controller reservation with its lifetime kernel lock. Dropping
/// it releases contention only: the durable UNKNOWN fence remains. This object
/// does not authenticate a worker or attest namespace/cgroup/provider confinement.
pub struct ReservedExecution {
    binding: ExecutionBinding,
    _lock: Flock<File>,
}

impl std::fmt::Debug for ReservedExecution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReservedExecution")
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl ReservedExecution {
    pub fn binding(&self) -> &ExecutionBinding {
        &self.binding
    }
}

fn private_directory(path: &Path) -> Result<File> {
    let directory = files::open_directory(path)?;
    let meta = directory.metadata()?;
    ensure!(
        meta.uid() == nix::unistd::geteuid().as_raw() && meta.mode() & 0o077 == 0,
        "execution store must be controller-owned and private"
    );
    Ok(directory)
}

fn locked_anchor(entry: &Path) -> Result<(Anchor, Flock<File>)> {
    let directory = private_directory(entry)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK | nix::libc::O_CLOEXEC)
        .open(entry.join("execution.lock"))?;
    files::private_file(&file)?;
    let lock = Flock::lock(file, FlockArg::LockExclusiveNonblock)
        .map_err(|(_, error)| anyhow::anyhow!("another controller owns this execution: {error}"))?;
    let directory = directory.metadata()?;
    let file = lock.metadata()?;
    Ok((
        Anchor {
            entry: entry.to_owned(),
            directory_device: directory.dev(),
            directory_inode: directory.ino(),
            lock_device: file.dev(),
            lock_inode: file.ino(),
            lock_identity: lock_identity(&lock, false)?,
        },
        lock,
    ))
}

fn read_journal(entry: &Path) -> Result<Journal> {
    let journal: Journal =
        serde_json::from_slice(&files::read_private(&entry.join("execution.json"))?)
            .context("invalid execution journal; retain UNKNOWN for diagnosis")?;
    ensure!(
        journal.schema_version == 2,
        "unsupported execution journal schema"
    );
    journal.fence.binding.validate()?;
    Ok(journal)
}

fn save(journal: &Journal) -> Result<()> {
    let entry = &journal.fence.anchor.entry;
    let mut temporary = tempfile::NamedTempFile::new_in(entry)?;
    temporary.write_all(&serde_json::to_vec(journal)?)?;
    files::private_file(temporary.as_file())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(entry.join("execution.json"))
        .map_err(|error| error.error)?;
    private_directory(entry)?.sync_all()?;
    Ok(())
}

impl ExecutionFence {
    pub fn binding(&self) -> &ExecutionBinding {
        &self.binding
    }

    /// Register under the original independent controller admission, before any
    /// backend invocation. Actual adapter inputs become available after daemon
    /// dispatch. Admission must already fence enqueue and dispatch; neither state
    /// loss nor ambiguous installation permits registering a replacement.
    pub(super) fn register_admitted(state: &Path, binding: &ExecutionBinding) -> Result<Self> {
        binding.validate()?;
        let root = private_directory(state)?;
        let entry = state.join(binding.request.key());
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&entry)
            .context("execution registration already exists or cannot be created")?;
        root.sync_all()?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
            .open(entry.join("execution.lock"))?;
        files::private_file(&file)?;
        lock_identity(&file, true)?;
        private_directory(&entry)?.sync_all()?;
        let (anchor, _lock) = locked_anchor(&entry)?;
        let fence = Self {
            anchor,
            binding: binding.clone(),
        };
        save(&Journal {
            schema_version: 2,
            fence: fence.clone(),
            state: ExecutionState::Ready,
        })?;
        Ok(fence)
    }

    /// Read only existing registration; never initialize, replace or rebind it.
    #[cfg(any(feature = "roborev-execution-tests", feature = "roborev-worker-tests"))]
    #[doc(hidden)]
    pub fn load(state: &Path, binding: &ExecutionBinding) -> Result<Self> {
        binding.validate()?;
        private_directory(state)?;
        let entry = state.join(binding.request.key());
        let (anchor, _lock) = locked_anchor(&entry)?;
        let journal = read_journal(&entry)?;
        ensure!(
            journal.fence.anchor == anchor,
            "execution registration anchor changed"
        );
        ensure!(
            journal.fence.binding == *binding,
            "execution request identity changed"
        );
        Ok(journal.fence)
    }

    fn checked(&self) -> Result<(Journal, Flock<File>)> {
        self.binding.validate()?;
        let (anchor, lock) = locked_anchor(&self.anchor.entry)?;
        ensure!(
            self.anchor == anchor,
            "execution registration anchor changed"
        );
        let journal = read_journal(&self.anchor.entry)?;
        ensure!(
            journal.fence == *self,
            "execution registration or binding changed"
        );
        Ok((journal, lock))
    }

    pub fn state(&self) -> Result<ExecutionState> {
        Ok(self.checked()?.0.state)
    }

    /// Persist UNKNOWN, fsync file and directory, then return exactly one possible
    /// execution reservation. Any error forbids backend invocation. Retries or
    /// restarts cannot reserve again, including when no backend actually started.
    pub(super) fn reserve_admitted(&self) -> Result<ReservedExecution> {
        let (mut journal, lock) = self.checked()?;
        ensure!(
            journal.state == ExecutionState::Ready,
            "backend execution is UNKNOWN; replay forbidden"
        );
        journal.state = ExecutionState::Unknown;
        save(&journal)?;
        Ok(ReservedExecution {
            binding: self.binding.clone(),
            _lock: lock,
        })
    }

    /// Native-fixture entrypoint; production must retain original Admission custody.
    #[cfg(any(feature = "roborev-execution-tests", feature = "roborev-worker-tests"))]
    #[doc(hidden)]
    pub fn register(state: &Path, binding: &ExecutionBinding) -> Result<Self> {
        Self::register_admitted(state, binding)
    }

    /// Native-fixture entrypoint; unavailable in the production feature set.
    #[cfg(any(feature = "roborev-execution-tests", feature = "roborev-worker-tests"))]
    #[doc(hidden)]
    pub fn reserve(&self) -> Result<ReservedExecution> {
        self.reserve_admitted()
    }
}
