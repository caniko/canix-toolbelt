//! Shared command arguments and value-returning dispatch; frontends own output and exit policy.

use super::*;
use clap::Subcommand;
use std::sync::atomic::AtomicBool;

/// Independent qualification command surface. JSON requests use the documented library types.
#[derive(Debug, Subcommand)]
pub enum QualificationCommand {
    /// Hash explicit source paths, or Git-selected tracked/unignored files
    Inventory {
        /// Source root
        #[arg(long)]
        root: PathBuf,
        /// Root-relative files/directories (required unless --git)
        #[arg(required_unless_present = "git", conflicts_with = "git")]
        paths: Vec<PathBuf>,
        /// Select files with Git, without recursing through gitlinks
        #[arg(long)]
        git: bool,
        /// Git executable for source selection
        #[arg(long, default_value = "git")]
        git_program: PathBuf,
    },
    /// Execute an explicit argument vector with source/input-bound logs and a finite deadline
    Run {
        /// RunRequest JSON
        #[arg(long)]
        request: PathBuf,
        /// New evidence directory (parent must exist)
        #[arg(long)]
        out: PathBuf,
    },
    /// Verify a completed Rust execution receipt and its retained artifacts
    VerifyRun {
        /// Evidence directory
        directory: PathBuf,
        /// Expected Composition JSON
        #[arg(long)]
        expected: Option<PathBuf>,
    },
    /// Verify historical Python receipt bytes; failed executions remain non-acceptance
    VerifyLegacyRun {
        /// Historical run directory
        directory: PathBuf,
    },
    /// Verify a portable artifact path-to-hash index, including retained Python indexes
    VerifyIndex {
        /// Artifact root
        #[arg(long)]
        root: PathBuf,
        /// ArtifactIndex JSON
        #[arg(long)]
        index: PathBuf,
    },
    /// Find direct dependency parents with target kinds, features and shortest workspace paths
    Parents {
        /// Cargo metadata --format-version 1 JSON
        metadata: PathBuf,
        /// Selected package name
        name: String,
        /// Optional exact selected version
        #[arg(long)]
        version: Option<String>,
    },
    /// Compare distribution-aware package identities in two resolved Cargo metadata documents
    GraphDelta {
        /// Baseline metadata JSON
        before: PathBuf,
        /// Candidate metadata JSON
        after: PathBuf,
    },
    /// Export deterministic selected source bytes to a new tar archive
    ExportSource {
        /// InventorySpec JSON
        #[arg(long)]
        request: PathBuf,
        /// New tar archive
        #[arg(long)]
        out: PathBuf,
    },
    /// Verify a source tar without extracting its paths
    VerifyArchive {
        /// Source tar
        archive: PathBuf,
        /// Expected Inventory JSON
        #[arg(long)]
        inventory: PathBuf,
    },
    /// Verify all required gates and seal a new portable handoff
    Seal {
        /// SealRequest JSON
        #[arg(long)]
        request: PathBuf,
        /// New handoff directory
        #[arg(long)]
        out: PathBuf,
    },
    /// Verify a handoff against independently supplied composition and required gate labels
    VerifySeal {
        /// Handoff directory
        directory: PathBuf,
        /// Expected Composition JSON
        #[arg(long)]
        expected: PathBuf,
        /// Independently selected gate-label/receipt-hash JSON map
        #[arg(long)]
        requirements: PathBuf,
    },
}

/// Dispatch result for frontend-owned JSON/human output and exit handling.
pub struct CommandResult {
    /// Structured evidence or result, without global output mutation.
    pub value: serde_json::Value,
    /// False for failed/cancelled/timed-out/source-changed or non-acceptable legacy runs.
    pub passed: bool,
}

fn result(value: &impl Serialize, passed: bool) -> io::Result<CommandResult> {
    Ok(CommandResult {
        value: serde_json::to_value(value).map_err(invalid_json)?,
        passed,
    })
}

/// Dispatch explicitly requested operations. Parsing a request alone never executes it.
pub fn dispatch(
    command: QualificationCommand,
    cancelled: &AtomicBool,
) -> io::Result<CommandResult> {
    match command {
        QualificationCommand::Inventory {
            root,
            paths,
            git,
            git_program,
        } => {
            let spec = if git {
                git_inventory_spec(&root, &git_program)?
            } else {
                InventorySpec { root, paths }
            };
            result(&inventory(&spec)?, true)
        }
        QualificationCommand::Run { request, out } => {
            let receipt = execute(&read_json(&request)?, &out, cancelled)?;
            result(&receipt, receipt.status == RunStatus::Passed)
        }
        QualificationCommand::VerifyRun {
            directory,
            expected,
        } => {
            let expected: Option<Composition> =
                expected.as_ref().map(|path| read_json(path)).transpose()?;
            result(&verify_run(&directory, expected.as_ref())?, true)
        }
        QualificationCommand::VerifyLegacyRun { directory } => {
            let verified = verify_legacy_run(&directory)?;
            result(&verified, verified.acceptable)
        }
        QualificationCommand::VerifyIndex { root, index } => result(
            &serde_json::json!({"artifactsVerified":verify_index(&root, &read_json(&index)?)?}),
            true,
        ),
        QualificationCommand::Parents {
            metadata,
            name,
            version,
        } => result(
            &graph::Metadata::parse(&fs::read(metadata)?)?.parents(&name, version.as_deref())?,
            true,
        ),
        QualificationCommand::GraphDelta { before, after } => {
            let before = graph::Metadata::parse(&fs::read(before)?)?;
            let after = graph::Metadata::parse(&fs::read(after)?)?;
            result(&before.delta(&after), true)
        }
        QualificationCommand::ExportSource { request, out } => {
            result(&export_source(&read_json(&request)?, &out)?, true)
        }
        QualificationCommand::VerifyArchive { archive, inventory } => {
            verify_source_archive(&archive, &read_json(&inventory)?)?;
            result(
                &serde_json::json!({"archiveSha256":sha256_file(&archive)?, "verified":true}),
                true,
            )
        }
        QualificationCommand::Seal { request, out } => {
            result(&seal(&read_json(&request)?, &out)?, true)
        }
        QualificationCommand::VerifySeal {
            directory,
            expected,
            requirements,
        } => result(
            &verify_seal(
                &directory,
                &read_json(&expected)?,
                &read_json(&requirements)?,
            )?,
            true,
        ),
    }
}
