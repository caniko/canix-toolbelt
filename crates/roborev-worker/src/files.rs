use super::budget::Budget;
use super::{Limits, is_hex};
use anyhow::{Context, Result, bail, ensure};
use nix::fcntl::{OFlag, openat};
use nix::sys::stat::Mode;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        },
    },
    path::{Component, Path},
};

fn normalized(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute()
            && path
                .components()
                .all(|part| matches!(part, Component::RootDir | Component::Normal(_))),
        "preparation path must be normalized and absolute"
    );
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) => ensure!(
                !meta.file_type().is_symlink(),
                "preparation path cannot traverse symlinks"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(super) fn private_directory(path: &Path) -> Result<()> {
    normalized(path)?;
    if !path.exists() {
        fs::DirBuilder::new().mode(0o700).create(path)?;
        File::open(
            path.parent()
                .context("preparation directory needs parent")?,
        )?
        .sync_all()?;
    }
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() && meta.uid() == nix::unistd::geteuid().as_raw() && meta.mode() & 0o077 == 0,
        "preparation state must be user-owned and private"
    );
    Ok(())
}

pub(super) fn private_file(file: &File) -> Result<()> {
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.nlink() == 1
            && meta.uid() == nix::unistd::geteuid().as_raw()
            && meta.mode() & 0o077 == 0,
        "preparation state file must be private, regular and unshared"
    );
    Ok(())
}

pub(super) fn read_private(path: &Path) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK | nix::libc::O_CLOEXEC)
        .open(path)?;
    private_file(&file)?;
    let mut bytes = Vec::new();
    file.take(65_537).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65_536, "oversized preparation journal");
    Ok(bytes)
}

pub(super) fn open_directory(path: &Path) -> Result<File> {
    ensure!(
        path.is_absolute(),
        "directory path must be normalized and absolute"
    );
    let mut directory = File::open("/")?;
    for part in path.components() {
        match part {
            Component::RootDir => {}
            Component::Normal(name) => {
                directory = File::from(openat(
                    &directory,
                    name,
                    OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
                    Mode::empty(),
                )?)
            }
            _ => bail!("directory path must be normalized and absolute"),
        }
    }
    Ok(directory)
}

fn child_file(parent: &File, name: &std::ffi::OsStr) -> Result<File> {
    Ok(File::from(openat(
        parent,
        name,
        OFlag::O_RDONLY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK,
        Mode::empty(),
    )?))
}

fn names(directory: &File, limit: usize, budget: &Budget) -> Result<Vec<std::ffi::OsString>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd()))? {
        budget.check()?;
        ensure!(
            names.len() < limit,
            "directory entry count exceeds preparation bound"
        );
        names.push(entry?.file_name());
    }
    names.sort();
    Ok(names)
}

pub(super) struct CopiedObjects {
    pub bytes: u64,
    pub entries: usize,
}

pub(super) fn copy_objects(
    source: &Path,
    target: &Path,
    limits: &Limits,
    budget: &Budget,
    hash_length: usize,
) -> Result<CopiedObjects> {
    normalized(source)?;
    let source = open_directory(source)?;
    let (mut bytes, mut count) = (0u64, 0usize);
    for directory in names(&source, limits.entries, budget)? {
        count += 1;
        ensure!(
            count <= limits.entries,
            "Git object count exceeds preparation bound"
        );
        let text = directory
            .to_str()
            .context("invalid object directory name")?;
        ensure!(
            text == "info" || text == "pack" || is_hex(text, 2),
            "unexpected object directory entry"
        );
        let child = child_file(&source, &directory)?;
        ensure!(
            child.metadata()?.is_dir(),
            "Git object shard must be a directory"
        );
        if text == "info" {
            let names = names(&child, limits.entries, budget)?;
            ensure!(
                !names
                    .iter()
                    .any(|name| name == "alternates" || name == "http-alternates"),
                "Git object alternates are forbidden"
            );
            continue;
        }
        let destination = target.join(&directory);
        if !destination.exists() {
            fs::DirBuilder::new().mode(0o700).create(&destination)?;
        }
        for name in names(&child, limits.entries, budget)? {
            let text_name = name.to_str().context("invalid Git object filename")?;
            let allowed = if text == "pack" {
                text_name
                    .strip_prefix("pack-")
                    .and_then(|name| {
                        name.strip_suffix(".pack")
                            .or_else(|| name.strip_suffix(".idx"))
                            .or_else(|| name.strip_suffix(".rev"))
                    })
                    .is_some_and(|hash| is_hex(hash, hash_length))
            } else {
                is_hex(text_name, hash_length - 2)
            };
            ensure!(allowed, "unsupported Git object storage file");
            count += 1;
            ensure!(
                count <= limits.entries,
                "Git object count exceeds preparation bound"
            );
            let mut input = child_file(&child, &name)
                .context("open regular Git object without following symlinks")?;
            let meta = input.metadata()?;
            ensure!(
                meta.is_file()
                    && meta.nlink() == 1
                    && meta.len() <= limits.input_bytes.saturating_sub(bytes),
                "Git object is not a bounded unshared regular file"
            );
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(destination.join(&name))?;
            let copied = std::io::copy(
                &mut Read::by_ref(&mut input).take(limits.input_bytes - bytes + 1),
                &mut output,
            )?;
            bytes = bytes
                .checked_add(copied)
                .context("object byte count overflow")?;
            ensure!(
                bytes <= limits.input_bytes && copied == meta.len(),
                "Git object changed or exceeded preparation bound"
            );
            output.flush()?;
            output.sync_all()?;
            budget.check()?;
        }
        File::open(destination)?.sync_all()?;
    }
    File::open(target)?.sync_all()?;
    Ok(CopiedObjects {
        bytes,
        entries: count,
    })
}

pub(super) fn neutralize_attributes(checkout: &Path) -> Result<()> {
    // Git 2.55 gitattributes(5): $GIT_DIR/info/attributes has highest
    // precedence, including over nested/index-backed .gitattributes. Keep raw
    // blob bytes rather than applying PR-controlled encodings/EOL/ident/filters.
    let directory = checkout.join(".git/info");
    private_directory(&directory)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join("attributes"))?;
    file.write_all(b"* -text -ident !filter !working-tree-encoding !eol\n")?;
    file.sync_all()?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

/// Hash raw path bytes, kind, sealed permissions, length and file/link bytes.
/// Symlink targets are data and are never opened. Descriptor-relative regular
/// file reads prevent a substituted symlink from traversing out of the snapshot.
pub(super) fn snapshot(
    root: &Path,
    limits: &Limits,
    budget: &Budget,
    seal: bool,
) -> Result<String> {
    normalized(root)?;
    fn walk(
        directory: File,
        relative: &Path,
        hash: &mut Sha256,
        counters: &mut (usize, u64),
        limits: &Limits,
        budget: &Budget,
        seal: bool,
    ) -> Result<()> {
        ensure!(
            relative.components().count() <= 128,
            "snapshot nesting exceeds preparation bound"
        );
        ensure!(
            directory.metadata()?.uid() == nix::unistd::geteuid().as_raw(),
            "snapshot directory owner changed"
        );
        for name in names(&directory, limits.entries, budget)? {
            counters.0 += 1;
            ensure!(
                counters.0 <= limits.entries,
                "snapshot entry count exceeds preparation bound"
            );
            let child = relative.join(&name);
            let path = Path::new("/proc/self/fd")
                .join(directory.as_raw_fd().to_string())
                .join(&name);
            let meta = fs::symlink_metadata(&path)?;
            ensure!(
                meta.uid() == nix::unistd::geteuid().as_raw(),
                "snapshot entry owner changed"
            );
            hash.update((child.as_os_str().as_bytes().len() as u64).to_le_bytes());
            hash.update(child.as_os_str().as_bytes());
            if meta.file_type().is_symlink() {
                hash.update(b"L");
                let target = fs::read_link(&path)?;
                hash.update((target.as_os_str().as_bytes().len() as u64).to_le_bytes());
                hash.update(target.as_os_str().as_bytes());
            } else {
                let file = child_file(&directory, &name)?;
                let opened = file.metadata()?;
                ensure!(
                    opened.ino() == meta.ino()
                        && opened.dev() == meta.dev()
                        && opened.uid() == nix::unistd::geteuid().as_raw(),
                    "snapshot identity changed during verification"
                );
                if opened.is_dir() {
                    hash.update(b"D");
                    walk(file, &child, hash, counters, limits, budget, seal)?;
                } else if opened.is_file() {
                    ensure!(opened.nlink() == 1, "snapshot contains shared hardlinks");
                    let mode = if opened.mode() & 0o111 != 0 {
                        0o500
                    } else {
                        0o400
                    };
                    if seal {
                        file.set_permissions(fs::Permissions::from_mode(mode))?;
                    }
                    ensure!(
                        file.metadata()?.mode() & 0o7777 == mode,
                        "snapshot file is not sealed"
                    );
                    counters.1 = counters
                        .1
                        .checked_add(opened.len())
                        .context("snapshot size overflow")?;
                    ensure!(
                        counters.1 <= limits.snapshot_bytes,
                        "snapshot exceeds preparation byte bound"
                    );
                    hash.update(b"F");
                    hash.update(mode.to_le_bytes());
                    hash.update(opened.len().to_le_bytes());
                    let mut input = file;
                    let mut buffer = [0u8; 65_536];
                    let mut read = 0u64;
                    loop {
                        budget.check()?;
                        let size = input.read(&mut buffer)?;
                        if size == 0 {
                            break;
                        }
                        read += size as u64;
                        ensure!(
                            read <= opened.len(),
                            "snapshot file grew during verification"
                        );
                        hash.update(&buffer[..size]);
                    }
                    ensure!(
                        read == opened.len(),
                        "snapshot file changed during verification"
                    );
                    input.sync_all()?;
                } else {
                    bail!("snapshot contains a special file");
                }
            }
        }
        if seal {
            directory.set_permissions(fs::Permissions::from_mode(0o500))?;
        }
        ensure!(
            directory.metadata()?.mode() & 0o7777 == 0o500,
            "snapshot directory is not sealed"
        );
        directory.sync_all()?;
        budget.check()?;
        Ok(())
    }
    let mut hash = Sha256::new();
    hash.update(b"canix-roborev-preparation-snapshot-v1\0");
    walk(
        open_directory(root)?,
        Path::new(""),
        &mut hash,
        &mut (0, 0),
        limits,
        budget,
        seal,
    )?;
    Ok(format!("{:x}", hash.finalize()))
}
