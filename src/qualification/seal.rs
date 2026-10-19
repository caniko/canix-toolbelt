use super::*;

/// Required gates and named artifacts for a portable qualification handoff.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SealRequest {
    /// Required immutable source/input identity, never inferred from a passing subset.
    pub composition: Composition,
    /// Required labels and completed execution directories. Must be nonempty.
    pub gates: BTreeMap<String, PathBuf>,
    /// Independently selected required label-to-receipt hashes. No passing substitutes.
    pub expected_receipts: ArtifactIndex,
    /// Additional files to retain; names are root-relative within artifacts/.
    #[serde(default)]
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
    use std::io::Write;
    let mut input = File::open(source)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    io::copy(&mut input, &mut output)?;
    output.flush()?;
    output.sync_all()?;
    let hash = sha256_file(destination)?;
    if sha256_file(source)? != hash {
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
        relative(Path::new(name))?;
    }
    fs::create_dir(directory)?;
    fs::create_dir(directory.join("gates"))?;
    fs::create_dir(directory.join("artifacts"))?;
    let mut artifacts = ArtifactIndex::new();
    let mut gates = ArtifactIndex::new();
    for (name, run) in &request.gates {
        fs::create_dir(directory.join("gates").join(name))?;
        for file in ["request.json", "receipt.json", "stdout.log", "stderr.log"] {
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
    let seal: Seal = read_json(&directory.join("seal.json"))?;
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
        let run = directory.join("gates").join(name);
        if sha256_file(&run.join("receipt.json"))? != *hash {
            return Err(invalid("sealed receipt hash differs"));
        }
        verify_run(&run, Some(expected))?;
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
    let output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)?;
    let mut builder = tar::Builder::new(output);
    for (name, source) in &before.files {
        let mut header = tar::Header::new_gnu();
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_mode(if source.executable { 0o755 } else { 0o644 });
        if let Some(link) = &source.link {
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_link_name(link)?;
            header.set_cksum();
            builder.append_data(&mut header, name, io::empty())?;
        } else {
            let file = File::open(root.join(name))?;
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

fn normalized_link(name: &str, link: &Path) -> io::Result<String> {
    if link.is_absolute() {
        return Err(invalid("absolute archive link"));
    }
    let mut parts: Vec<_> = Path::new(name)
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .map(|p| p.as_os_str().to_owned())
        .collect();
    for part in link.components() {
        match part {
            Component::Normal(part) => parts.push(part.to_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(invalid("archive link escapes root"));
                }
            }
            _ => return Err(invalid("invalid archive link")),
        }
    }
    path_text(&parts.iter().collect::<PathBuf>())
}

/// Verify tar source bytes and filesystem semantics without extracting untrusted paths.
/// Duplicate paths, hard links, devices, escaping links, and cyclic links are rejected.
pub fn verify_source_archive(archive: &Path, expected: &Inventory) -> io::Result<()> {
    if !matches!(expected.schema_version, 1 | 2) || expected.files.is_empty() {
        return Err(invalid("invalid expected archive inventory"));
    }
    if archive_inventory(archive, expected.schema_version)? != expected.files {
        return Err(invalid("archive inventory differs from qualified source"));
    }
    Ok(())
}

/// Verify the historical Python path/content-hash map against a source tar.
/// This content-only check does not establish executable-bit or link-text parity.
pub fn verify_legacy_archive(archive: &Path, expected: &ArtifactIndex) -> io::Result<usize> {
    if expected.is_empty() {
        return Err(invalid("legacy archive inventory is empty"));
    }
    let actual: ArtifactIndex = archive_inventory(archive, 1)?
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

fn archive_inventory(archive: &Path, version: u32) -> io::Result<BTreeMap<String, SourceFile>> {
    let mut tar = tar::Archive::new(File::open(archive)?);
    let mut files = BTreeMap::new();
    let mut links = BTreeMap::new();
    let mut seen_paths = std::collections::BTreeSet::new();
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        relative(&path)?;
        let name = path_text(&path)?;
        if !seen_paths.insert(path.components().collect::<PathBuf>()) {
            return Err(invalid("duplicate archive path"));
        }
        let executable = entry.header().mode()? & 0o111 != 0;
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
        if kind.is_dir() {
            continue;
        }
        let mut hash = Sha256::new();
        let link = if kind.is_symlink() {
            let link = entry
                .link_name()?
                .ok_or_else(|| invalid("missing archive link target"))?
                .into_owned();
            links.insert(name.clone(), normalized_link(&name, &link)?);
            let link = path_text(&link)?;
            if version == 2 {
                hash.update(link.as_bytes());
            }
            Some(link)
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
    for path in &seen_paths {
        for ancestor in path.ancestors().skip(1) {
            if links.contains_key(&path_text(ancestor)?) {
                return Err(invalid("archive entry is beneath a symlink"));
            }
        }
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
        if let Some(file) = files.get(target) {
            if version == 1 {
                let sha256 = file.sha256.clone();
                files
                    .get_mut(name)
                    .ok_or_else(|| invalid("archive symlink entry is absent"))?
                    .sha256 = sha256;
            }
        } else if version != 2 || !files.keys().any(|path| Path::new(path).starts_with(target)) {
            return Err(invalid("archive symlink target is absent"));
        } else if links.keys().any(|path| Path::new(path).starts_with(target)) {
            // ponytail: directory-link targets must have symlink-free subtrees.
            // Supporting recursive aliases requires graph-aware cycle resolution.
            return Err(invalid(
                "archive directory symlink target contains symlinks",
            ));
        }
    }
    Ok(files)
}
