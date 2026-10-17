//! Bounded, descriptor-selected JSON shared with the private namespace executor.
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub(crate) fn invalid_json(error: serde_json::Error) -> io::Error {
    invalid(error.to_string())
}

pub(crate) fn regular_file(path: &Path) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Validate the opened descriptor, not a racy path stat. In particular,
        // opening a FIFO must never wait for a writer before we can reject it.
        options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
    }
    #[cfg(not(unix))]
    if !fs::metadata(path)?.is_file() {
        return Err(invalid("not a regular file"));
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid(format!("not a regular file: {}", path.display())));
    }
    Ok(file)
}

// Parse exactly the bytes whose independently selected digest was checked.
// Reopening the path after hashing would allow another attempt to substitute
// its passing document. Bound the read even if a concurrent writer grows it.
pub(crate) fn read_bound_json<T: serde::de::DeserializeOwned>(
    path: &Path,
    expected: Option<&str>,
) -> io::Result<T> {
    read_bound_json_file(regular_file(path)?, expected)
}

pub(super) fn read_bound_json_file<T: serde::de::DeserializeOwned>(
    file: File,
    expected: Option<&str>,
) -> io::Result<T> {
    serde_json::from_slice(&read_bound_bytes(file, expected)?).map_err(invalid_json)
}

pub(super) fn read_bound_bytes(file: File, expected: Option<&str>) -> io::Result<Vec<u8>> {
    let before = file.metadata()?;
    if before.len() > 64 * 1024 * 1024 {
        return Err(invalid("evidence JSON exceeds 64 MiB"));
    }
    let mut bytes = Vec::new();
    (&file).take(before.len() + 1).read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    if bytes.len() as u64 != before.len()
        || after.len() != before.len()
        || before.modified()? != after.modified()?
    {
        return Err(invalid("evidence JSON changed while reading"));
    }
    if expected.is_some_and(|hash| format!("{:x}", Sha256::digest(&bytes)) != hash) {
        return Err(invalid("selected evidence JSON digest differs"));
    }
    Ok(bytes)
}
