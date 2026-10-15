//! Bounded retention for the private tmpfs work tree, after producer drainage.
use std::{
    fs,
    io::{self, Read, Write},
    path::Path,
};

pub const MAX_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 16384;

pub fn copy_tree(
    source: &Path,
    destination: &Path,
    cancelled: &impl Fn() -> bool,
) -> io::Result<()> {
    let mut pending = vec![(source.to_path_buf(), destination.to_path_buf())];
    let mut entries = 0;
    let mut bytes = 0u64;
    while let Some((source, destination)) = pending.pop() {
        fs::create_dir_all(&destination)?;
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
            if let Ok(existing) = fs::symlink_metadata(&target) {
                if !existing.is_dir() {
                    fs::remove_file(&target)?;
                } else if !metadata.is_dir() {
                    return Err(io::Error::other("work retention type changed"));
                }
            }
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
    Ok(())
}
