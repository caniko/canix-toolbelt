use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyReceipt {
    #[serde(rename = "exit")]
    exit: i32,
    execution_exit: Option<i32>,
    source_unchanged: bool,
    source_sha256: String,
    stdout_sha256: Option<String>,
    stderr_sha256: Option<String>,
    log_sha256: Option<String>,
    lock_sha256: Option<String>,
}

/// Integrity result for historical Python receipts, distinct from required-gate acceptance.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyVerification {
    /// Exit zero and unchanged source according to the historical receipt.
    /// Historical tool/environment/input gaps remain; this is not a new-format gate.
    pub acceptable: bool,
    /// Original child exit, before source-change override if recorded.
    pub execution_exit: i32,
    /// Number of hashed portable artifacts verified.
    pub artifacts_verified: usize,
    /// Historical source.json byte identity.
    pub source_sha256: String,
}

/// Verify retained Python source/log/lock bytes without promoting their provenance contract.
/// Optional historical external-input/tool hashes need their own artifact-index verification.
pub fn verify_legacy_run(directory: &Path) -> io::Result<LegacyVerification> {
    let root = directory.canonicalize()?;
    let receipt: LegacyReceipt = read_json(&contained(&root, Path::new("receipt.json"))?)?;
    let mut index = ArtifactIndex::from([("source.json".into(), receipt.source_sha256.clone())]);
    match (
        receipt.stdout_sha256,
        receipt.stderr_sha256,
        receipt.log_sha256,
    ) {
        (Some(out), Some(err), None) => {
            index.insert("stdout.log".into(), out);
            index.insert("stderr.log".into(), err);
        }
        (None, None, Some(log)) => {
            index.insert("output.log".into(), log);
        }
        _ => {
            return Err(invalid(
                "legacy receipt has missing or ambiguous log hashes",
            ));
        }
    }
    if let Some(hash) = receipt.lock_sha256 {
        index.insert("Cargo.lock".into(), hash);
    }
    let artifacts_verified = verify_index(&root, &index)?;
    Ok(LegacyVerification {
        acceptable: receipt.exit == 0
            && receipt.execution_exit.is_none_or(|exit| exit == 0)
            && receipt.source_unchanged,
        execution_exit: receipt.execution_exit.unwrap_or(receipt.exit),
        artifacts_verified,
        source_sha256: receipt.source_sha256,
    })
}
