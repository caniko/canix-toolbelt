//! Complete offline backend output retained under independent controller custody.
//! Admission owns installation uncertainty and the original serialized result.
//! A retained backend result is not daemon completion or forge acceptance.

use super::{execution::ExecutionBinding, files, offline::OfflineReceipt};
use anyhow::{Context, Result, ensure};
use nix::{
    fcntl::{OFlag, openat},
    sys::stat::Mode,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
const RECEIPT_LIMIT: usize = 256 * 1024;

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Member {
    device: u64,
    inode: u64,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RetainedCompletion {
    entry: PathBuf,
    directory_device: u64,
    directory_inode: u64,
    binding: ExecutionBinding,
    stdout: Member,
    stderr: Member,
    receipt: Member,
}

fn private_directory(path: &Path) -> Result<File> {
    let directory = files::open_directory(path)?;
    let meta = directory.metadata()?;
    ensure!(
        meta.uid() == nix::unistd::geteuid().as_raw() && meta.mode() & 0o077 == 0,
        "completion custody must be controller-owned and private"
    );
    Ok(directory)
}

fn read(
    directory: &File,
    name: &str,
    maximum: usize,
    expected: Option<&Member>,
) -> Result<Vec<u8>> {
    let file = File::from(openat(
        directory,
        name,
        OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?);
    files::private_file(&file)?;
    let meta = file.metadata()?;
    ensure!(
        meta.len() <= maximum as u64,
        "oversized retained completion member"
    );
    if let Some(expected) = expected {
        ensure!(
            (meta.dev(), meta.ino()) == (expected.device, expected.inode),
            "original completion member changed"
        );
    }
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() == meta.len() as usize && bytes.len() <= maximum,
        "incomplete completion member"
    );
    if let Some(expected) = expected {
        ensure!(
            digest(&bytes) == expected.sha256,
            "retained completion bytes changed"
        );
    }
    Ok(bytes)
}

fn write(entry: &Path, name: &str, bytes: &[u8]) -> Result<Member> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(entry.join(name))?;
    files::private_file(&file)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    let meta = file.metadata()?;
    Ok(Member {
        device: meta.dev(),
        inode: meta.ino(),
        sha256: digest(bytes),
    })
}

fn check_receipt(
    receipt: &OfflineReceipt,
    binding: &ExecutionBinding,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<()> {
    ensure!(
        receipt.scope == "offline-envelope-only; not forge acceptance or daemon authentication"
            && receipt.binding == *binding
            && receipt.backend_success
            && receipt.cgroup_empty,
        "completion is not the original successful, cleaned-up offline execution"
    );
    ensure!(
        !stdout.is_empty()
            && stdout.len() + stderr.len() <= OUTPUT_LIMIT
            && receipt.stdout_sha256 == digest(stdout)
            && receipt.stderr_sha256 == digest(stderr),
        "completion output is incomplete or differs from original receipt"
    );
    Ok(())
}

impl RetainedCompletion {
    pub(super) fn binding(&self) -> &ExecutionBinding {
        &self.binding
    }

    /// The caller must have independently obtained this receipt from run_offline,
    /// retained its original result directory and committed installation UNKNOWN.
    /// Matching fields are integrity checks; deserialized backend claims cannot
    /// establish successful execution or confinement.
    pub(super) fn retain(
        root: &Path,
        binding: &ExecutionBinding,
        result: &Path,
        expected: &OfflineReceipt,
    ) -> Result<Self> {
        let source = private_directory(result)?;
        let receipt = read(&source, "receipt.json", RECEIPT_LIMIT, None)?;
        let observed: OfflineReceipt = serde_json::from_slice(&receipt)?;
        ensure!(
            serde_json::to_vec(&observed)? == serde_json::to_vec(expected)?,
            "original worker completion receipt changed"
        );
        let stdout = read(&source, "stdout", OUTPUT_LIMIT, None)?;
        let stderr = read(&source, "stderr", OUTPUT_LIMIT, None)?;
        check_receipt(&observed, binding, &stdout, &stderr)?;
        let root_directory = private_directory(root)?;
        let entry = root.join(binding.request.key());
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&entry)
            .context("completion already exists or cannot be retained; replacement forbidden")?;
        root_directory.sync_all()?;
        let directory = private_directory(&entry)?;
        let stdout = write(&entry, "stdout", &stdout)?;
        let stderr = write(&entry, "stderr", &stderr)?;
        let receipt = write(&entry, "receipt.json", &receipt)?;
        directory.sync_all()?;
        let meta = directory.metadata()?;
        Ok(Self {
            entry,
            directory_device: meta.dev(),
            directory_inode: meta.ino(),
            binding: binding.clone(),
            stdout,
            stderr,
            receipt,
        })
    }

    /// Read through the ORIGINAL controller anchor, bounded and descriptor-relative.
    /// Call only after complete input equivalence and authenticated same-job delivery.
    pub(super) fn output(&self, binding: &ExecutionBinding) -> Result<Vec<u8>> {
        ensure!(
            self.binding == *binding,
            "redelivery input/backend/request/job identity differs"
        );
        let directory = private_directory(&self.entry)?;
        let meta = directory.metadata()?;
        ensure!(
            (meta.dev(), meta.ino()) == (self.directory_device, self.directory_inode),
            "original completion directory changed"
        );
        let stdout = read(&directory, "stdout", OUTPUT_LIMIT, Some(&self.stdout))?;
        let stderr = read(&directory, "stderr", OUTPUT_LIMIT, Some(&self.stderr))?;
        let receipt: OfflineReceipt = serde_json::from_slice(&read(
            &directory,
            "receipt.json",
            RECEIPT_LIMIT,
            Some(&self.receipt),
        )?)?;
        check_receipt(&receipt, binding, &stdout, &stderr)?;
        Ok(stdout)
    }
}
