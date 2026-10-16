//! Bounded retention for the private tmpfs work tree, after producer drainage.
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
};

pub const MAX_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 16384;

pub fn copy_tree(
    source: &Path,
    destination: &Path,
    cancelled: &impl Fn() -> bool,
) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    // Each probe/command starts from the preceding retained tree. Replacing its
    // children preserves deletions and type changes instead of resurrecting
    // producer-removed files. Never follow a retained child symlink.
    let mut pending = vec![destination.to_path_buf()];
    let mut cleared = Vec::new();
    while let Some(directory) = pending.pop() {
        if cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        let mut permissions = fs::metadata(&directory)?.permissions();
        permissions.set_mode(permissions.mode() | 0o700);
        fs::set_permissions(&directory, permissions)?;
        for entry in fs::read_dir(&directory)? {
            if cancelled() {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            let entry = entry?;
            if fs::symlink_metadata(entry.path())?.is_dir() {
                pending.push(entry.path());
            } else {
                fs::remove_file(entry.path())?;
            }
        }
        cleared.push(directory);
    }
    for directory in cleared.into_iter().rev() {
        if cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        if directory != destination {
            fs::remove_dir(directory)?;
        }
    }
    let mut pending = vec![(source.to_path_buf(), destination.to_path_buf())];
    let mut entries = 0;
    let mut bytes = 0u64;
    let mut directories = Vec::new();
    while let Some((source, destination)) = pending.pop() {
        fs::create_dir_all(&destination)?;
        directories.push((destination.clone(), fs::metadata(&source)?.permissions()));
        for entry in fs::read_dir(source)? {
            if cancelled() {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            entries += 1;
            if entries > MAX_ENTRIES {
                return Err(io::Error::other("work retention inode budget exceeded"));
            }
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            let target = destination.join(entry.file_name());
            if metadata.is_dir() {
                pending.push((entry.path(), target));
            } else if metadata.file_type().is_symlink() {
                std::os::unix::fs::symlink(fs::read_link(entry.path())?, target)?;
            } else if metadata.is_file() {
                bytes = bytes
                    .checked_add(metadata.len())
                    .filter(|size| *size <= MAX_BYTES)
                    .ok_or_else(|| io::Error::other("work retention byte budget exceeded"))?;
                let mut input = fs::File::open(entry.path())?.take(metadata.len());
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)?;
                let mut buffer = [0u8; 65536];
                let mut copied = 0;
                loop {
                    if cancelled() {
                        return Err(io::Error::from(io::ErrorKind::Interrupted));
                    }
                    let count = input.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    output.write_all(&buffer[..count])?;
                    copied += count as u64;
                }
                if copied != metadata.len() {
                    return Err(io::Error::other("work retention short read"));
                }
                fs::set_permissions(target, metadata.permissions())?;
            } else {
                return Err(io::Error::other("work retention rejects special files"));
            }
        }
    }
    // Apply restrictive modes only after creating descendants, children first.
    for (directory, permissions) in directories.into_iter().rev() {
        if cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        fs::set_permissions(directory, permissions)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        copy_tree(&source, &retained, &|| false).unwrap();
        copy_tree(&retained, &restored, &|| false).unwrap();
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
