//! Bounded retention for the private tmpfs work tree, after producer drainage.
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::Path,
};

pub const MAX_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 16384;

pub fn open_directory(path: &Path) -> io::Result<File> {
    Ok(rustix::fs::openat2(
        rustix::fs::CWD,
        path,
        directory_flags(),
        rustix::fs::Mode::empty(),
        rustix::fs::ResolveFlags::NO_SYMLINKS,
    )?
    .into())
}

fn directory_flags() -> rustix::fs::OFlags {
    rustix::fs::OFlags::RDONLY
        | rustix::fs::OFlags::DIRECTORY
        | rustix::fs::OFlags::NOFOLLOW
        | rustix::fs::OFlags::CLOEXEC
        | rustix::fs::OFlags::NOATIME
}

fn child_directory(parent: &File, name: &std::ffi::OsStr) -> io::Result<File> {
    Ok(rustix::fs::openat(parent, name, directory_flags(), rustix::fs::Mode::empty())?.into())
}

fn children(
    directory: &File,
    remaining: usize,
    cancelled: &impl Fn() -> bool,
) -> io::Result<Vec<std::ffi::OsString>> {
    let mut names = Vec::new();
    // read_from preserves NOATIME and independently rewinds the pinned handle.
    for entry in rustix::fs::Dir::read_from(directory)? {
        if cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let entry = entry?;
        if matches!(entry.file_name().to_bytes(), b"." | b"..") {
            continue;
        }
        if names.len() >= remaining {
            return Err(io::Error::other("work retention inode budget exceeded"));
        }
        names.push(std::ffi::OsStr::from_bytes(entry.file_name().to_bytes()).to_owned());
    }
    names.sort();
    Ok(names)
}

#[cfg(test)]
pub fn copy_tree(
    source: &Path,
    destination: &Path,
    cancelled: &impl Fn() -> bool,
) -> io::Result<String> {
    fs::create_dir_all(destination)?;
    copy_tree_opened(
        &open_directory(source)?,
        &open_directory(destination)?,
        cancelled,
    )
}

pub fn copy_tree_opened(
    source: &File,
    destination: &File,
    cancelled: &impl Fn() -> bool,
) -> io::Result<String> {
    use rustix::fs::{
        AtFlags, Mode, OFlags, fchmod, mkdirat, openat, readlinkat, statat, symlinkat, unlinkat,
    };
    reject_xattrs(rustix::fs::flistxattr(source, &mut [0u8; 0][..]))?;
    reject_xattrs(rustix::fs::flistxattr(destination, &mut [0u8; 0][..]))?;
    // Each probe/command starts from the preceding retained tree. Replacing its
    // children preserves deletions and type changes instead of resurrecting
    // producer-removed files. Never follow a retained child symlink.
    let mut pending = vec![(destination.try_clone()?, None)];
    let mut cleared = Vec::new();
    let mut entries = 0;
    while let Some((directory, parent)) = pending.pop() {
        if cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        fchmod(
            &directory,
            Mode::from_raw_mode(directory.metadata()?.mode() | 0o700),
        )?;
        for name in children(&directory, MAX_ENTRIES - entries, cancelled)? {
            if cancelled() {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            entries += 1;
            let metadata = statat(&directory, &name, AtFlags::SYMLINK_NOFOLLOW)?;
            if rustix::fs::FileType::from_raw_mode(metadata.st_mode)
                == rustix::fs::FileType::Directory
            {
                pending.push((
                    child_directory(&directory, &name)?,
                    Some((directory.try_clone()?, name)),
                ));
            } else {
                unlinkat(&directory, &name, AtFlags::empty())?;
            }
        }
        cleared.push((directory, parent));
    }
    for (_directory, parent) in cleared.into_iter().rev() {
        if cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        if let Some((parent, name)) = parent {
            unlinkat(&parent, name, AtFlags::REMOVEDIR)?;
        }
    }
    let mut pending = vec![(
        source.try_clone()?,
        destination.try_clone()?,
        std::path::PathBuf::new(),
    )];
    let mut entries = 0;
    let mut bytes = 0u64;
    let mut directories = Vec::new();
    let mut hash = Sha256::new();
    hash.update(b"qualification-work-v2\0");
    while let Some((source, destination, path)) = pending.pop() {
        reject_xattrs(rustix::fs::flistxattr(&source, &mut [0u8; 0][..]))?;
        let metadata = source.metadata()?;
        hash.update(b"d");
        digest_part(&mut hash, path.as_os_str().as_bytes());
        digest_metadata(&mut hash, &metadata);
        directories.push((destination.try_clone()?, metadata));
        for name in children(&source, MAX_ENTRIES - entries, cancelled)? {
            if cancelled() {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            entries += 1;
            if entries > MAX_ENTRIES {
                return Err(io::Error::other("work retention inode budget exceeded"));
            }
            let metadata = statat(&source, &name, AtFlags::SYMLINK_NOFOLLOW)?;
            let kind = rustix::fs::FileType::from_raw_mode(metadata.st_mode);
            let path = path.join(&name);
            if kind == rustix::fs::FileType::Directory {
                mkdirat(&destination, &name, Mode::RWXU)?;
                pending.push((
                    child_directory(&source, &name)?,
                    child_directory(&destination, &name)?,
                    path,
                ));
            } else if kind == rustix::fs::FileType::Symlink {
                use std::os::fd::AsRawFd;
                // Resolve only the pinned parent descriptor; never follow the
                // producer's link, including links outside the work tree.
                let metadata_path = Path::new("/proc/self/fd")
                    .join(source.as_raw_fd().to_string())
                    .join(&name);
                reject_xattrs(rustix::fs::llistxattr(metadata_path, &mut [0u8; 0][..]))?;
                if metadata.st_nlink != 1 {
                    return Err(io::Error::other(
                        "work retention rejects hard-linked symlinks",
                    ));
                }
                let link = readlinkat(&source, &name, Vec::new())?;
                let times = rustix::fs::Timestamps {
                    last_access: rustix::fs::Timespec {
                        tv_sec: metadata.st_atime,
                        tv_nsec: metadata.st_atime_nsec as _,
                    },
                    last_modification: rustix::fs::Timespec {
                        tv_sec: metadata.st_mtime,
                        tv_nsec: metadata.st_mtime_nsec as _,
                    },
                };
                // readlink updates atime even when the directory uses NOATIME.
                rustix::fs::utimensat(&source, &name, &times, AtFlags::SYMLINK_NOFOLLOW)?;
                hash.update(b"l");
                digest_part(&mut hash, path.as_os_str().as_bytes());
                hash.update((metadata.st_mode & 0o7777).to_le_bytes());
                digest_times(&mut hash, &times);
                digest_part(&mut hash, link.to_bytes());
                symlinkat(&link, &destination, &name)?;
                rustix::fs::utimensat(&destination, &name, &times, AtFlags::SYMLINK_NOFOLLOW)?;
            } else if kind == rustix::fs::FileType::RegularFile {
                let input = File::from(openat(
                    &source,
                    &name,
                    OFlags::RDONLY
                        | OFlags::NOFOLLOW
                        | OFlags::NONBLOCK
                        | OFlags::CLOEXEC
                        | OFlags::NOATIME,
                    Mode::empty(),
                )?);
                let metadata = input.metadata()?;
                reject_xattrs(rustix::fs::flistxattr(&input, &mut [0u8; 0][..]))?;
                if !metadata.is_file() {
                    return Err(io::Error::other("work file changed type"));
                }
                if metadata.nlink() != 1 {
                    return Err(io::Error::other(
                        "work retention rejects hard links instead of changing inode aliases",
                    ));
                }
                if metadata.len() != 0 {
                    // Work v2 cannot authenticate extent layout. Refuse holes
                    // rather than materializing them on the next private tmpfs.
                    let hole = rustix::fs::seek(&input, rustix::fs::SeekFrom::Hole(0))?;
                    if hole < metadata.len() {
                        return Err(io::Error::other("work retention rejects sparse files"));
                    }
                    rustix::fs::seek(&input, rustix::fs::SeekFrom::Start(0))?;
                }
                bytes = bytes
                    .checked_add(metadata.len())
                    .filter(|size| *size <= MAX_BYTES)
                    .ok_or_else(|| io::Error::other("work retention byte budget exceeded"))?;
                let mut input = input.take(metadata.len());
                let mut output = File::from(openat(
                    &destination,
                    &name,
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC,
                    Mode::RUSR | Mode::WUSR,
                )?);
                let mut buffer = [0u8; 65536];
                let mut copied = 0;
                hash.update(b"f");
                digest_part(&mut hash, path.as_os_str().as_bytes());
                digest_metadata(&mut hash, &metadata);
                hash.update(metadata.len().to_le_bytes());
                loop {
                    if cancelled() {
                        return Err(io::Error::from(io::ErrorKind::Interrupted));
                    }
                    let count = input.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    output.write_all(&buffer[..count])?;
                    hash.update(&buffer[..count]);
                    copied += count as u64;
                }
                if copied != metadata.len() {
                    return Err(io::Error::other("work retention short read"));
                }
                output.set_permissions(metadata.permissions())?;
                rustix::fs::futimens(&output, &timestamps(&metadata))?;
            } else {
                return Err(io::Error::other("work retention rejects special files"));
            }
        }
    }
    // Apply restrictive modes only after creating descendants, children first.
    for (directory, metadata) in directories.into_iter().rev() {
        if cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        directory.set_permissions(metadata.permissions())?;
        rustix::fs::futimens(&directory, &timestamps(&metadata))?;
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn reject_xattrs(result: rustix::io::Result<usize>) -> io::Result<()> {
    match result {
        Ok(0) | Err(rustix::io::Errno::NOTSUP) => Ok(()),
        Ok(_) => Err(io::Error::other(
            "work retention rejects extended attributes and ACLs",
        )),
        Err(error) => Err(error.into()),
    }
}

fn timestamps(metadata: &fs::Metadata) -> rustix::fs::Timestamps {
    rustix::fs::Timestamps {
        last_access: rustix::fs::Timespec {
            tv_sec: metadata.atime(),
            tv_nsec: metadata.atime_nsec(),
        },
        last_modification: rustix::fs::Timespec {
            tv_sec: metadata.mtime(),
            tv_nsec: metadata.mtime_nsec(),
        },
    }
}

fn digest_times(hash: &mut Sha256, times: &rustix::fs::Timestamps) {
    for time in [times.last_access, times.last_modification] {
        hash.update(time.tv_sec.to_le_bytes());
        hash.update(time.tv_nsec.to_le_bytes());
    }
}

fn digest_metadata(hash: &mut Sha256, metadata: &fs::Metadata) {
    hash.update((metadata.mode() & 0o7777).to_le_bytes());
    digest_times(hash, &timestamps(metadata));
}

fn digest_part(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn late_review_sparse_work_files_are_rejected_instead_of_densified() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        let mut file = File::create(source.join("sparse")).unwrap();
        file.set_len(1024 * 1024).unwrap();
        file.write_all(b"header").unwrap();
        let result = copy_tree(&source, &root.path().join("retained"), &|| false);
        assert!(
            result.is_err(),
            "sparse work transition silently changed extent semantics"
        );
    }

    #[test]
    fn final_review_work_xattrs_and_acls_are_never_silently_dropped() {
        // Linux validates this native POSIX ACL value, rather than treating it
        // as a user.* lookalike: version 2, owner/named user/group/mask/other.
        let mut acl = 2u32.to_le_bytes().to_vec();
        for (tag, permissions, id) in [
            (1u16, 7u16, u32::MAX),
            (2, 4, rustix::process::geteuid().as_raw()),
            (4, 5, u32::MAX),
            (16, 5, u32::MAX),
            (32, 0, u32::MAX),
        ] {
            acl.extend(tag.to_le_bytes());
            acl.extend(permissions.to_le_bytes());
            acl.extend(id.to_le_bytes());
        }
        for (name, attribute) in [
            ("", "user.qualification"),
            ("nested", "user.qualification"),
            ("nested/file", "user.qualification"),
            ("", "system.posix_acl_access"),
            ("nested", "system.posix_acl_default"),
            ("nested/file", "system.posix_acl_access"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source");
            fs::create_dir_all(source.join("nested")).unwrap();
            fs::write(source.join("nested/file"), b"cache").unwrap();
            let value = if attribute.starts_with("system.") {
                acl.as_slice()
            } else {
                b"metadata"
            };
            let stored = match rustix::fs::setxattr(
                source.join(name),
                attribute,
                value,
                rustix::fs::XattrFlags::empty(),
            ) {
                Ok(()) => true,
                // Nix's seccomp filter forbids both creation and enumeration of
                // EAs/ACLs. Test the metadata-free copy there; native CI must
                // create every real fixture and exercise its rejection instead.
                Err(rustix::io::Errno::NOTSUP)
                    if std::env::var_os("NIX_BUILD_TOP").is_some()
                        && std::env::var_os("CANIX_QUALIFICATION_REQUIRE_XATTRS").is_none() =>
                {
                    assert_eq!(
                        rustix::fs::flistxattr(
                            open_directory(&source).unwrap(),
                            &mut [0u8; 0][..]
                        ),
                        Err(rustix::io::Errno::NOTSUP)
                    );
                    false
                }
                Err(error) => panic!("cannot create required {attribute} on {name:?}: {error}"),
            };
            let result = copy_tree(&source, &root.path().join("retained"), &|| false);
            if stored {
                let error = result.expect_err("silently discarded work metadata");
                assert!(
                    error.to_string().contains("extended attributes and ACLs"),
                    "{attribute} on {name:?}: {error}"
                );
            } else {
                result.expect("metadata-free work must survive Nix's xattr filter");
            }
        }
    }

    #[test]
    fn new_review_work_timestamps_survive_and_bind_both_copy_directions() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested/file"), b"cache").unwrap();
        std::os::unix::fs::symlink("file", source.join("nested/link")).unwrap();
        let times = rustix::fs::Timestamps {
            last_access: rustix::fs::Timespec {
                tv_sec: 123,
                tv_nsec: 456,
            },
            last_modification: rustix::fs::Timespec {
                tv_sec: 789,
                tv_nsec: 123,
            },
        };
        for path in ["nested/file", "nested/link", "nested", ""] {
            rustix::fs::utimensat(
                rustix::fs::CWD,
                source.join(path),
                &times,
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
            )
            .unwrap();
        }
        let retained = root.path().join("retained");
        let restored = root.path().join("restored");
        let selected = copy_tree(&source, &retained, &|| false).unwrap();
        assert_eq!(
            selected,
            copy_tree(&retained, &restored, &|| false).unwrap()
        );
        assert_eq!(
            selected,
            copy_tree(&retained, &root.path().join("restored-again"), &|| false).unwrap()
        );
        for path in ["nested/file", "nested/link", "nested", ""] {
            let metadata = fs::symlink_metadata(restored.join(path)).unwrap();
            assert_eq!(
                (
                    metadata.atime(),
                    metadata.atime_nsec(),
                    metadata.mtime(),
                    metadata.mtime_nsec()
                ),
                (123, 456, 789, 123),
                "{path}"
            );
        }
        let changed = rustix::fs::Timestamps {
            last_modification: rustix::fs::Timespec {
                tv_sec: 790,
                tv_nsec: 123,
            },
            ..times
        };
        rustix::fs::utimensat(
            rustix::fs::CWD,
            retained.join("nested/file"),
            &changed,
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        )
        .unwrap();
        assert_ne!(
            selected,
            copy_tree(&retained, &root.path().join("changed"), &|| false).unwrap()
        );
    }

    #[test]
    fn new_review_retention_never_traverses_destination_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        let victim = root.path().join("victim");
        for path in [&source, &destination, &victim] {
            fs::create_dir(path).unwrap();
        }
        fs::write(victim.join("keep"), b"unrelated").unwrap();
        std::os::unix::fs::symlink(&victim, destination.join("nested")).unwrap();
        copy_tree(&source, &destination, &|| false).unwrap();
        assert_eq!(fs::read(victim.join("keep")).unwrap(), b"unrelated");
    }

    #[test]
    fn retention_preserves_directory_modes_in_both_directions() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let retained = root.path().join("retained");
        let restored = root.path().join("restored");
        fs::create_dir_all(source.join("private")).unwrap();
        fs::write(source.join("private/file"), b"artifact").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o710)).unwrap();
        fs::set_permissions(source.join("private"), fs::Permissions::from_mode(0o700)).unwrap();
        let selected = copy_tree(&source, &retained, &|| false).unwrap();
        assert_eq!(
            selected,
            copy_tree(&retained, &restored, &|| false).unwrap()
        );
        for destination in [&retained, &restored] {
            assert_eq!(
                fs::metadata(destination).unwrap().permissions().mode() & 0o7777,
                0o710
            );
            assert_eq!(
                fs::metadata(destination.join("private"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o7777,
                0o700
            );
            assert_eq!(
                fs::read(destination.join("private/file")).unwrap(),
                b"artifact"
            );
        }
    }

    #[test]
    fn retained_read_only_directories_can_be_replaced() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let retained = root.path().join("retained");
        fs::create_dir_all(source.join("readonly")).unwrap();
        fs::write(source.join("readonly/old"), b"old").unwrap();
        fs::set_permissions(source.join("readonly"), fs::Permissions::from_mode(0o555)).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o555)).unwrap();
        copy_tree(&source, &retained, &|| false).unwrap();
        copy_tree(&source, &retained, &|| false).unwrap();
        assert_eq!(
            fs::metadata(&retained).unwrap().permissions().mode() & 0o7777,
            0o555
        );
        assert_eq!(
            fs::metadata(retained.join("readonly"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o555
        );
        assert_eq!(fs::read(retained.join("readonly/old")).unwrap(), b"old");
    }

    #[test]
    fn retention_reflects_deletions_and_directory_to_file_transitions() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let destination = root.path().join("retained");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("removed"), b"probe artifact").unwrap();
        fs::write(source.join("nested/old"), b"old").unwrap();
        copy_tree(&source, &destination, &|| false).unwrap();
        fs::remove_file(source.join("removed")).unwrap();
        fs::remove_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested"), b"replacement").unwrap();
        copy_tree(&source, &destination, &|| false).unwrap();
        assert!(!destination.join("removed").exists());
        assert_eq!(
            fs::read(destination.join("nested")).unwrap(),
            b"replacement"
        );
    }
}
