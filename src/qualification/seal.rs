use super::*;

/// Required gates and named artifacts for a portable qualification handoff.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SealRequest {
    /// Required immutable source/input identity, never inferred from a passing subset.
    pub composition: Composition,
    /// Required labels and completed execution directories. Must be nonempty.
    #[serde(deserialize_with = "unique_map")]
    pub gates: BTreeMap<String, PathBuf>,
    /// Independently selected required label-to-receipt hashes. No passing substitutes.
    pub expected_receipts: ArtifactIndex,
    /// Additional files to retain; names are root-relative within artifacts/.
    #[serde(default, deserialize_with = "unique_map")]
    pub artifacts: BTreeMap<String, PathBuf>,
}

/// Portable, integrity-checked handoff. Not signed and not a deployment authorization.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Seal {
    /// Wire version, currently 1.
    pub schema_version: u32,
    /// Exact qualified composition.
    pub composition: Composition,
    /// Required gate labels and their retained receipt hashes.
    pub gates: ArtifactIndex,
    /// Hashes of all retained artifacts except this self-describing seal.
    pub artifacts: ArtifactIndex,
}

fn gate_label(name: &str) -> io::Result<()> {
    relative(Path::new(name))?;
    if name == "." || name.contains(['/', '\\']) {
        return Err(invalid("gate labels must be single path components"));
    }
    Ok(())
}

fn copy(source: &Path, destination: &Path) -> io::Result<String> {
    let input = regular_file(source)?;
    let before = input.metadata()?;
    copy_opened(input, before, destination)
}

fn copy_opened(input: File, before: fs::Metadata, destination: &Path) -> io::Result<String> {
    use std::io::Write;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut bounded = (&input).take(before.len());
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    let mut copied = 0;
    loop {
        let count = bounded.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count])?;
        hash.update(&buffer[..count]);
        copied += count as u64;
    }
    let after = input.metadata()?;
    if copied != before.len()
        || before.len() != after.len()
        || before.modified()? != after.modified()?
    {
        return Err(invalid("artifact changed during bounded copy"));
    }
    output.flush()?;
    output.sync_all()?;
    let hash = format!("{:x}", hash.finalize());
    if sha256_file(destination)? != hash {
        return Err(invalid("artifact changed during copy"));
    }
    Ok(hash)
}

/// Verify every required gate, then copy selected bytes into a new handoff directory.
/// A failure leaves incomplete evidence without seal.json; it cannot satisfy verification.
pub fn seal(request: &SealRequest, directory: &Path) -> io::Result<Seal> {
    if request.gates.is_empty() {
        return Err(invalid("a seal requires a nonempty explicit gate set"));
    }
    if request.gates.keys().ne(request.expected_receipts.keys()) {
        return Err(invalid(
            "gate selection differs from the required receipt set",
        ));
    }
    for (name, run) in &request.gates {
        gate_label(name)?;
        verify_run(run, Some(&request.composition))?;
        if sha256_file(&run.join("receipt.json"))? != request.expected_receipts[name] {
            return Err(invalid(format!(
                "required attempt differs for gate: {name}"
            )));
        }
    }
    for name in request.artifacts.keys() {
        artifact_key(name)?;
    }
    fs::create_dir(directory)?;
    fs::create_dir(directory.join("gates"))?;
    fs::create_dir(directory.join("artifacts"))?;
    let mut artifacts = ArtifactIndex::new();
    let mut gates = ArtifactIndex::new();
    for (name, run) in &request.gates {
        fs::create_dir(directory.join("gates").join(name))?;
        for file in [
            "request.json",
            "boundary.json",
            "receipt.json",
            "execution.json",
            "stdout.log",
            "stderr.log",
        ] {
            let relative = format!("gates/{name}/{file}");
            let hash = copy(&run.join(file), &directory.join(&relative))?;
            if file == "receipt.json" {
                gates.insert(name.clone(), hash.clone());
            }
            artifacts.insert(relative, hash);
        }
        verify_run(
            &directory.join("gates").join(name),
            Some(&request.composition),
        )?;
    }
    for (name, source) in &request.artifacts {
        let relative = format!("artifacts/{name}");
        let destination = directory.join(&relative);
        fs::create_dir_all(
            destination
                .parent()
                .ok_or_else(|| invalid("artifact has no parent"))?,
        )?;
        artifacts.insert(relative, copy(source, &destination)?);
    }
    let seal = Seal {
        schema_version: 1,
        composition: request.composition.clone(),
        gates,
        artifacts,
    };
    if seal.gates != request.expected_receipts {
        return Err(invalid("required receipt changed during seal"));
    }
    verify_index(directory, &seal.artifacts)?;
    write_json(&directory.join("seal.json"), &seal)?;
    Ok(seal)
}

/// Verify a portable handoff against an externally supplied composition and exact required gates.
/// Requiring these separately prevents a handoff from silently omitting failing checks.
pub fn verify_seal(
    directory: &Path,
    expected: &Composition,
    required: &ArtifactIndex,
) -> io::Result<Seal> {
    let root = directory.canonicalize()?;
    let directory = &root;
    let seal: Seal = read_json(&evidence_path(&root, Path::new("seal.json"))?)?;
    if seal.schema_version != 1
        || &seal.composition != expected
        || required.is_empty()
        || required != &seal.gates
    {
        return Err(invalid(
            "seal has a different composition or required gate set",
        ));
    }
    verify_index(directory, &seal.artifacts)?;
    for (name, hash) in &seal.gates {
        gate_label(name)?;
        for file in [
            "request.json",
            "boundary.json",
            "receipt.json",
            "execution.json",
            "stdout.log",
            "stderr.log",
        ] {
            let path = format!("gates/{name}/{file}");
            if !seal.artifacts.contains_key(&path) {
                return Err(invalid(format!(
                    "mandatory gate artifact is not indexed: {path}"
                )));
            }
            evidence_path(&root, Path::new(&path))?;
        }
        let run = evidence_path(&root, &Path::new("gates").join(name))?;
        super::receipt::verify_run_bound(&run, Some(expected), Some(hash))?;
    }
    Ok(seal)
}

/// Export selected source as a deterministic tar, with executable bits and relative links.
/// File contents are checked again after export; output must be outside selected directories.
pub fn export_source(spec: &InventorySpec, archive: &Path) -> io::Result<Inventory> {
    let before = inventory(spec)?;
    let root = spec.root.canonicalize()?;
    let parent = archive
        .parent()
        .ok_or_else(|| invalid("archive has no parent"))?
        .canonicalize()?;
    let destination = parent.join(
        archive
            .file_name()
            .ok_or_else(|| invalid("archive has no name"))?,
    );
    for path in &spec.paths {
        let path = contained(&root, path)?;
        if path.is_dir() && destination.starts_with(&path) {
            return Err(invalid("archive is inside selected source"));
        }
    }
    let mut directories = std::collections::BTreeSet::new();
    for (name, source) in &before.files {
        add_parent_directories(name, &mut directories)?;
        if let Some(link) = &source.link {
            let (_, traversed) = link_walk(name, Path::new(link))?;
            directories.extend(traversed);
        }
    }
    for name in &directories {
        // Lexical link walking is valid only for real directories. A directory
        // symlink can change the meaning of a following '..'; reject that case.
        if !fs::symlink_metadata(root.join(name))?.is_dir() {
            return Err(invalid(
                "source link traverses a non-directory or directory symlink",
            ));
        }
    }
    let output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)?;
    let mut builder = tar::Builder::new(output);
    for name in &directories {
        let mut header = tar::Header::new_gnu();
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::Directory);
        header.set_size(0);
        header.set_cksum();
        builder.append_data(&mut header, name, io::empty())?;
    }
    for (name, source) in &before.files {
        let mut header = tar::Header::new_gnu();
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_mode(if source.executable { 0o755 } else { 0o644 });
        if let Some(link) = &source.link {
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            builder.append_link(&mut header, name, link)?;
        } else {
            let file = regular_file(&root.join(name))?;
            header.set_size(file.metadata()?.len());
            header.set_cksum();
            builder.append_data(&mut header, name, file)?;
        }
    }
    builder.into_inner()?.sync_all()?;
    if inventory(spec)? != before {
        return Err(invalid("source changed during archive export"));
    }
    verify_source_archive(&destination, &before)?;
    Ok(before)
}

fn add_parent_directories(
    name: &str,
    directories: &mut std::collections::BTreeSet<String>,
) -> io::Result<()> {
    let mut parent = Path::new(name).parent();
    while let Some(path) = parent.filter(|path| !path.as_os_str().is_empty()) {
        directories.insert(path_text(path)?);
        parent = path.parent();
    }
    Ok(())
}

fn link_walk(name: &str, link: &Path) -> io::Result<(String, std::collections::BTreeSet<String>)> {
    if link.is_absolute() {
        return Err(invalid("absolute archive link"));
    }
    let mut parts: Vec<_> = Path::new(name)
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .map(|p| p.as_os_str().to_owned())
        .collect();
    let mut directories = std::collections::BTreeSet::new();
    add_parent_directories(name, &mut directories)?;
    let mut components = link.components().peekable();
    while let Some(part) = components.next() {
        match part {
            Component::Normal(part) => {
                parts.push(part.to_owned());
                if components.peek().is_some() {
                    directories.insert(path_text(&parts.iter().collect::<PathBuf>())?);
                }
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(invalid("archive link escapes root"));
                }
            }
            _ => return Err(invalid("invalid archive link")),
        }
    }
    Ok((path_text(&parts.iter().collect::<PathBuf>())?, directories))
}

/// Verify tar source bytes and filesystem semantics without extracting untrusted paths.
/// Duplicate paths, hard links, devices, escaping links, and cyclic links are rejected.
pub fn verify_source_archive(archive: &Path, expected: &Inventory) -> io::Result<()> {
    verify_source_archive_sha256(archive, expected).map(|_| ())
}

/// Verify inventory and hash the exact byte stream parsed from one regular-file descriptor.
pub fn verify_source_archive_sha256(archive: &Path, expected: &Inventory) -> io::Result<String> {
    if expected.schema_version != 1 || expected.files.is_empty() {
        return Err(invalid("invalid expected archive inventory"));
    }
    let (files, hash) = archive_inventory(archive)?;
    if files != expected.files {
        return Err(invalid("archive inventory differs from qualified source"));
    }
    Ok(hash)
}

/// Verify the historical Python path/content-hash map against a source tar.
/// This content-only check does not establish executable-bit or link-text parity.
pub fn verify_legacy_archive(archive: &Path, expected: &ArtifactIndex) -> io::Result<usize> {
    if expected.is_empty() {
        return Err(invalid("legacy archive inventory is empty"));
    }
    let actual: ArtifactIndex = archive_inventory(archive)?
        .0
        .into_iter()
        .map(|(name, file)| (name, file.sha256))
        .collect();
    if actual != *expected {
        return Err(invalid(
            "legacy archive content differs from qualified source",
        ));
    }
    Ok(actual.len())
}

fn archive_inventory(archive: &Path) -> io::Result<(BTreeMap<String, SourceFile>, String)> {
    archive_inventory_from_file(regular_file(archive)?)
}

fn archive_inventory_from_file(file: File) -> io::Result<(BTreeMap<String, SourceFile>, String)> {
    struct HashingReader<R> {
        input: R,
        hash: Sha256,
        count: u64,
    }
    impl<R: Read> Read for HashingReader<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = self.input.read(buffer)?;
            self.hash.update(&buffer[..count]);
            self.count += count as u64;
            Ok(count)
        }
    }
    let before = file.metadata()?;
    let bound = before
        .len()
        .checked_add(1)
        .ok_or_else(|| invalid("archive length cannot be bounded"))?;
    let mut reader = HashingReader {
        input: (&file).take(bound),
        hash: Sha256::new(),
        count: 0,
    };
    let mut tar = tar::Archive::new(&mut reader);
    let mut files = BTreeMap::new();
    let mut links = BTreeMap::new();
    let mut directories = std::collections::BTreeSet::new();
    let mut seen_paths = std::collections::BTreeSet::new();
    for entry in tar.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() {
            // Git archive adds a commit comment. Never ignore global path/size
            // overrides, which tar readers may apply to every following file.
            if entry.size() > 1024 {
                return Err(invalid("oversized Git archive metadata"));
            }
            let extensions = entry
                .pax_extensions()?
                .ok_or_else(|| invalid("missing Git archive metadata"))?;
            for extension in extensions {
                let extension = extension?;
                let value = extension.value_bytes();
                if extension.key_bytes() != b"comment"
                    || !matches!(value.len(), 40 | 64)
                    || !value.iter().all(u8::is_ascii_hexdigit)
                {
                    return Err(invalid("unsupported global archive metadata"));
                }
            }
            continue;
        }
        let path = entry.path()?.into_owned();
        relative(&path)?;
        let name = path_text(&path)?.trim_start_matches("./").to_owned();
        if !seen_paths.insert(name.clone()) {
            return Err(invalid("duplicate archive path"));
        }
        let executable = entry.header().mode()? & 0o111 != 0;
        add_parent_directories(&name, &mut directories)?;
        if kind.is_dir() {
            directories.insert(name);
            continue;
        }
        let mut hash = Sha256::new();
        let link = if kind.is_symlink() {
            let link = entry
                .link_name()?
                .ok_or_else(|| invalid("missing archive link target"))?
                .into_owned();
            links.insert(name.clone(), link.clone());
            Some(path_text(&link)?)
        } else if kind.is_file() {
            let mut buffer = [0; 65536];
            loop {
                let n = entry.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                hash.update(&buffer[..n]);
            }
            None
        } else {
            return Err(invalid("archive contains unsupported entry type"));
        };
        files.insert(
            name,
            SourceFile {
                sha256: format!("{:x}", hash.finalize()),
                executable,
                link,
            },
        );
    }
    if files.keys().any(|name| directories.contains(name)) {
        return Err(invalid("archive file is also required as a directory"));
    }
    let mut represented_directories = std::collections::BTreeSet::new();
    for name in files.keys() {
        add_parent_directories(name, &mut represented_directories)?;
    }
    let links: BTreeMap<_, _> = links
        .iter()
        .map(|(name, link)| {
            let (target, traversed) = link_walk(name, link)?;
            if !traversed.is_subset(&directories) {
                return Err(invalid(
                    "archive omits a directory required by a link target",
                ));
            }
            represented_directories.extend(traversed);
            Ok((name.clone(), target))
        })
        .collect::<io::Result<_>>()?;
    // Ignore only the archive's structural root, not extra visible directories
    // that the qualified file/link inventory cannot describe.
    directories.remove(".");
    represented_directories.remove(".");
    if directories != represented_directories {
        return Err(invalid("archive contains unrepresented empty directories"));
    }
    for (name, target) in &links {
        let mut target = target;
        let mut seen = std::collections::BTreeSet::new();
        while let Some(next) = links.get(target) {
            if !seen.insert(target) {
                return Err(invalid("cyclic archive symlink"));
            }
            target = next;
        }
        let target_file = files
            .get(target)
            .ok_or_else(|| invalid("archive symlink target is absent"))?
            .clone();
        let file = files
            .get_mut(name)
            .ok_or_else(|| invalid("archive symlink entry is absent"))?;
        file.sha256 = target_file.sha256;
        file.executable = target_file.executable;
    }
    io::copy(tar.into_inner(), &mut io::sink())?;
    let after = file.metadata()?;
    if reader.count != before.len()
        || before.len() != after.len()
        || before.modified()? != after.modified()?
    {
        return Err(invalid("archive changed during verification"));
    }
    Ok((files, format!("{:x}", reader.hash.finalize())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growing_artifact_copies_never_read_beyond_the_opened_size() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("artifact");
        let destination = root.path().join("copy");
        fs::write(&source, b"original").unwrap();
        let input = regular_file(&source).unwrap();
        let before = input.metadata().unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&source)
            .unwrap()
            .set_len(1 << 40)
            .unwrap();
        assert!(copy_opened(input, before, &destination).is_err());
        assert_eq!(fs::read(destination).unwrap(), b"original");
    }

    #[test]
    fn parsed_archive_hash_stays_bound_when_the_path_is_replaced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.tar");
        let mut builder = tar::Builder::new(File::create(&path).unwrap());
        let mut header = tar::Header::new_gnu();
        header.set_size(8);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "source", &b"original"[..])
            .unwrap();
        builder.finish().unwrap();
        drop(builder);
        // Include bytes after tar EOF: the returned identity covers the entire
        // archive descriptor, including padding not interpreted as source.
        use std::io::Write;
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"trailing bytes")
            .unwrap();
        let original_hash = sha256_file(&path).unwrap();
        let file = regular_file(&path).unwrap();
        fs::rename(&path, root.path().join("original.tar")).unwrap();
        fs::write(&path, b"replacement that was not verified").unwrap();
        let (files, verified_hash) = archive_inventory_from_file(file).unwrap();
        assert_eq!(verified_hash, original_hash);
        assert_ne!(verified_hash, sha256_file(&path).unwrap());
        assert_eq!(
            files["source"].sha256,
            format!("{:x}", Sha256::digest(b"original"))
        );
    }
}
