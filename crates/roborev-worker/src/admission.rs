//! Independent Canix controller admission, preceding daemon and backend effects.
//! This store and its serialized original handle must stay outside daemon/backend
//! write authority. These transitions are accounting, not process authentication,
//! an upstream dispatch engine, input verification or a confinement receipt.

use super::{
    Anchor, Binding,
    completion::RetainedCompletion,
    execution::{ExecutionBinding, ExecutionFence, ReservedExecution},
    files, is_hex, lock_identity,
    offline::OfflineReceipt,
};
use anyhow::{Context, Result, ensure};
use nix::fcntl::{Flock, FlockArg};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
};

/// Original admitted request and request-exclusive daemon/database custody.
/// The daemon manifest identifies stable database/config/endpoint authority;
/// each live process/unit generation must be authenticated separately.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionBinding {
    pub request: Binding,
    pub controller_id: String,
    pub daemon_identity_sha256: String,
}

impl AdmissionBinding {
    fn validate(&self) -> Result<()> {
        self.request.validate()?;
        ensure!(
            is_hex(&self.controller_id, 64) && is_hex(&self.daemon_identity_sha256, 64),
            "admission requires original controller and daemon custody identities"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobIdentity {
    pub id: u64,
    pub uuid: String,
}

impl JobIdentity {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.id > 0
                && self.uuid.len() == 36
                && self.uuid.split('-').map(str::len).eq([8, 4, 4, 4, 12])
                && self.uuid.split('-').all(|part| is_hex(part, part.len())),
            "admission requires a persisted exact job ID and UUID"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionPhase {
    Admitted,
    EnqueueUnknown,
    JobBound,
    DispatchUnknown,
    ExecutionRegistrationUnknown,
    ExecutionRegistered,
    CompletionRetentionUnknown,
    CompletionRetained,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum State {
    Admitted,
    EnqueueUnknown,
    JobBound {
        job: JobIdentity,
    },
    DispatchUnknown {
        job: JobIdentity,
    },
    ExecutionRegistrationUnknown {
        binding: ExecutionBinding,
    },
    ExecutionRegistered {
        fence: ExecutionFence,
    },
    CompletionRetentionUnknown {
        binding: ExecutionBinding,
    },
    CompletionRetained {
        fence: ExecutionFence,
        result: Box<RetainedCompletion>,
    },
}

/// Preserve this original anchor in independent wider controller custody before
/// enqueue. Recovery operates on this handle, never on a newly inferred anchor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Admission {
    anchor: Anchor,
    binding: AdmissionBinding,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: u32,
    admission: Admission,
    state: State,
}

fn private_directory(path: &Path) -> Result<File> {
    let directory = files::open_directory(path)?;
    let meta = directory.metadata()?;
    ensure!(
        meta.uid() == nix::unistd::geteuid().as_raw() && meta.mode() & 0o077 == 0,
        "admission store must be controller-owned and private"
    );
    Ok(directory)
}

fn locked_anchor(entry: &Path) -> Result<(Anchor, Flock<File>)> {
    let directory = private_directory(entry)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK | nix::libc::O_CLOEXEC)
        .open(entry.join("admission.lock"))?;
    files::private_file(&file)?;
    let lock = Flock::lock(file, FlockArg::LockExclusiveNonblock)
        .map_err(|(_, error)| anyhow::anyhow!("another controller owns admission: {error}"))?;
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

fn save(journal: &Journal) -> Result<()> {
    let entry = &journal.admission.anchor.entry;
    let mut temporary = tempfile::NamedTempFile::new_in(entry)?;
    temporary.write_all(&serde_json::to_vec(journal)?)?;
    files::private_file(temporary.as_file())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(entry.join("admission.json"))
        .map_err(|error| error.error)?;
    private_directory(entry)?.sync_all()?;
    Ok(())
}

impl Admission {
    pub fn binding(&self) -> &AdmissionBinding {
        &self.binding
    }

    /// The job recorded under ORIGINAL request custody, never inferred from a
    /// peer claim or a search for currently running rows.
    pub fn dispatched_job(&self) -> Result<JobIdentity> {
        let (journal, _lock) = self.checked()?;
        match journal.state {
            State::DispatchUnknown { job } => Ok(job),
            State::ExecutionRegistrationUnknown { binding } => Ok(JobIdentity {
                id: binding.job_id,
                uuid: binding.job_uuid,
            }),
            State::ExecutionRegistered { fence } => Ok(JobIdentity {
                id: fence.binding().job_id,
                uuid: fence.binding().job_uuid.clone(),
            }),
            State::CompletionRetentionUnknown { binding } => Ok(JobIdentity {
                id: binding.job_id,
                uuid: binding.job_uuid,
            }),
            State::CompletionRetained { fence, .. } => Ok(JobIdentity {
                id: fence.binding().job_id,
                uuid: fence.binding().job_uuid.clone(),
            }),
            _ => anyhow::bail!("adapter delivery requires original persisted dispatch admission"),
        }
    }

    /// Fresh authorized admission only. The caller must own durable request
    /// freshness; disappearance of the wider ledger cannot authorize this method.
    /// Partial registration remains a tombstone, never reset or overwritten.
    pub fn register(state: &Path, binding: &AdmissionBinding) -> Result<Self> {
        binding.validate()?;
        let root = private_directory(state)?;
        let entry = state.join(binding.request.key());
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&entry)
            .context("admission already exists or cannot be created")?;
        root.sync_all()?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
            .open(entry.join("admission.lock"))?;
        files::private_file(&file)?;
        lock_identity(&file, true)?;
        private_directory(&entry)?.sync_all()?;
        let (anchor, _lock) = locked_anchor(&entry)?;
        let admission = Self {
            anchor,
            binding: binding.clone(),
        };
        save(&Journal {
            schema_version: 1,
            admission: admission.clone(),
            state: State::Admitted,
        })?;
        Ok(admission)
    }

    fn checked(&self) -> Result<(Journal, Flock<File>)> {
        self.binding.validate()?;
        let (anchor, lock) = locked_anchor(&self.anchor.entry)?;
        ensure!(anchor == self.anchor, "original admission anchor changed");
        let journal: Journal = serde_json::from_slice(&files::read_private(
            &self.anchor.entry.join("admission.json"),
        )?)
        .context("invalid admission journal; retain UNKNOWN")?;
        ensure!(
            journal.schema_version == 1 && journal.admission == *self,
            "original admission schema or binding changed"
        );
        match &journal.state {
            State::JobBound { job } | State::DispatchUnknown { job } => job.validate()?,
            State::ExecutionRegistrationUnknown { binding }
            | State::CompletionRetentionUnknown { binding } => {
                self.check_execution(binding, None)?
            }
            State::ExecutionRegistered { fence } => self.check_execution(fence.binding(), None)?,
            State::CompletionRetained { fence, result } => {
                self.check_execution(fence.binding(), None)?;
                ensure!(
                    result.binding() == fence.binding(),
                    "retained completion differs from original execution"
                );
            }
            State::Admitted | State::EnqueueUnknown => {}
        }
        Ok((journal, lock))
    }

    pub fn phase(&self) -> Result<AdmissionPhase> {
        Ok(match self.checked()?.0.state {
            State::Admitted => AdmissionPhase::Admitted,
            State::EnqueueUnknown => AdmissionPhase::EnqueueUnknown,
            State::JobBound { .. } => AdmissionPhase::JobBound,
            State::DispatchUnknown { .. } => AdmissionPhase::DispatchUnknown,
            State::ExecutionRegistrationUnknown { .. } => {
                AdmissionPhase::ExecutionRegistrationUnknown
            }
            State::ExecutionRegistered { .. } => AdmissionPhase::ExecutionRegistered,
            State::CompletionRetentionUnknown { .. } => AdmissionPhase::CompletionRetentionUnknown,
            State::CompletionRetained { .. } => AdmissionPhase::CompletionRetained,
        })
    }

    /// Fsync UNKNOWN before the single enqueue effect. A lost response retains
    /// UNKNOWN even if the server did not create a job; this never grants a retry.
    /// First prove a fresh empty request-exclusive daemon and persisted queue pause.
    pub fn begin_enqueue(&self) -> Result<()> {
        let (mut journal, _lock) = self.checked()?;
        ensure!(
            matches!(journal.state, State::Admitted),
            "enqueue UNKNOWN or already bound; replay forbidden"
        );
        journal.state = State::EnqueueUnknown;
        save(&journal)
    }

    /// Record a checked response or reconcile a lost response against the complete
    /// unfiltered single-job inventory of the ORIGINAL exclusive daemon/database.
    /// Caller must validate range/type/policy and reject any extra job or truncation.
    /// This is not permission to infer a job from matching active rows or timestamps.
    pub fn bind_job(&self, job: &JobIdentity) -> Result<()> {
        job.validate()?;
        let (mut journal, _lock) = self.checked()?;
        match &journal.state {
            State::EnqueueUnknown => {
                journal.state = State::JobBound { job: job.clone() };
                save(&journal)
            }
            State::JobBound { job: original } if original == job => Ok(()),
            _ => anyhow::bail!("job cannot replace original admission or precede enqueue"),
        }
    }

    /// Commit dispatch uncertainty before queue release. The immutable adapter
    /// endpoint remains fail-closed until this state and the original job are
    /// durable. Lost unpause replies reconcile queue state, never re-enqueue.
    pub fn begin_dispatch(&self) -> Result<()> {
        let (mut journal, _lock) = self.checked()?;
        let State::JobBound { job } = &journal.state else {
            anyhow::bail!("dispatch requires original durable job; replay forbidden");
        };
        journal.state = State::DispatchUnknown { job: job.clone() };
        save(&journal)
    }

    fn check_execution(&self, binding: &ExecutionBinding, job: Option<&JobIdentity>) -> Result<()> {
        binding.validate()?;
        ensure!(
            binding.request == self.binding.request
                && binding.controller_id == self.binding.controller_id
                && binding.daemon_identity_sha256 == self.binding.daemon_identity_sha256,
            "execution differs from original admitted request/controller/daemon"
        );
        if let Some(job) = job {
            ensure!(
                binding.job_id == job.id && binding.job_uuid == job.uuid,
                "execution differs from original persisted job"
            );
        }
        Ok(())
    }

    /// Called only after authenticated adapter delivery and complete verified input
    /// custody. Bind actual inputs now, after queue release and before backend start.
    /// Persist installation UNKNOWN before creating the one-shot execution fence.
    /// Any error or crash during installation forbids replacement registration.
    pub fn register_execution(
        &self,
        state: &Path,
        binding: &ExecutionBinding,
    ) -> Result<ExecutionFence> {
        let (mut journal, _lock) = self.checked()?;
        let State::DispatchUnknown { job } = &journal.state else {
            anyhow::bail!("execution registration is not fresh; retain UNKNOWN");
        };
        self.check_execution(binding, Some(job))?;
        journal.state = State::ExecutionRegistrationUnknown {
            binding: binding.clone(),
        };
        save(&journal)?;
        let fence = ExecutionFence::register_admitted(state, binding)?;
        journal.state = State::ExecutionRegistered {
            fence: fence.clone(),
        };
        save(&journal)?;
        Ok(fence)
    }

    /// Original persisted admission and fence are both required. Missing state is
    /// never recreated, even if roborev has requeued the same job after restart.
    pub fn reserve_execution(&self) -> Result<ReservedExecution> {
        let (journal, _lock) = self.checked()?;
        let State::ExecutionRegistered { fence } = journal.state else {
            anyhow::bail!("no confirmed original execution registration; retain UNKNOWN");
        };
        fence.reserve_admitted()
    }

    /// Retain a successful independently verified OFFLINE envelope result before
    /// exposing output. The original run_offline receipt/result custody is required;
    /// an untrusted worker-supplied receipt cannot authorize this transition.
    /// Installation failures remain UNKNOWN and cannot be re-created on recovery.
    /// This does not establish daemon completion, production execution or acceptance.
    pub fn retain_offline_completion(
        &self,
        state: &Path,
        result: &Path,
        receipt: &OfflineReceipt,
    ) -> Result<()> {
        let (mut journal, _lock) = self.checked()?;
        let State::ExecutionRegistered { fence } = &journal.state else {
            anyhow::bail!("completion is not fresh; retain UNKNOWN");
        };
        let fence = fence.clone();
        ensure!(
            receipt.binding == *fence.binding(),
            "completion differs from original execution"
        );
        ensure!(
            fence.state()? == super::execution::ExecutionState::Unknown,
            "completion requires original consumed execution"
        );
        journal.state = State::CompletionRetentionUnknown {
            binding: fence.binding().clone(),
        };
        save(&journal)?;
        let retained = RetainedCompletion::retain(state, fence.binding(), result, receipt)?;
        journal.state = State::CompletionRetained {
            fence,
            result: Box::new(retained),
        };
        save(&journal)
    }

    /// Controller-only output read after authenticated invocation of the ORIGINAL
    /// persisted job and re-verification of its complete actual input/backend binding.
    /// Only a currently running adapter delivery is eligible. Failed/cancelled/done
    /// jobs never obtain output or a new execution. Daemon observation and transport
    /// authentication stay caller obligations; this method performs no daemon effects.
    pub fn retained_offline_output(
        &self,
        binding: &ExecutionBinding,
        job: &JobIdentity,
        running: bool,
    ) -> Result<Vec<u8>> {
        ensure!(
            running,
            "terminal or non-running jobs cannot receive retained output"
        );
        let (journal, _lock) = self.checked()?;
        let State::CompletionRetained { fence, result } = journal.state else {
            anyhow::bail!("complete original backend output unavailable; retain UNKNOWN");
        };
        self.check_execution(binding, Some(job))?;
        ensure!(
            fence.binding() == binding
                && fence.state()? == super::execution::ExecutionState::Unknown,
            "original consumed execution fence changed"
        );
        result.output(binding)
    }
}
