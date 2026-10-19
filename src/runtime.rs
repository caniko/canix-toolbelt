//! Typed runtime-manifest loading with consumer-owned paths.
//!
//! Pkl evaluation is embedded through Pklx. Loading does not execute listed
//! binaries, connect to endpoints, or open referenced secret files.

use miette::IntoDiagnostic;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A binary path on the target host.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinPath {
    /// Logical binary name.
    pub name: String,
    /// Consumer-supplied executable path.
    pub path: String,
    /// Optional human-readable description.
    pub description: Option<String>,
}

/// A service endpoint the consumer may need to reach.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceEndpoint {
    /// Logical endpoint name.
    pub name: String,
    /// Protocol scheme supplied by the manifest producer.
    pub scheme: String,
    /// Hostname or address.
    pub host: String,
    /// Service port.
    pub port: u16,
    /// Optional human-readable description.
    pub description: Option<String>,
}

/// A reference to an agenix-provisioned file, never its contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretRef {
    /// Logical secret name.
    pub name: String,
    /// Path to the provisioned secret (`agenixPath` on the wire).
    pub agenix_path: String,
    /// Producer-declared file mode, retained as a string.
    pub mode: String,
}

/// Runtime facts supplied by the consuming deployment.
///
/// All three mappings are required. Pkl schemas may supply defaults; the Rust
/// loader does not silently turn incomplete or invalid documents into empties.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeManifest {
    /// Named executable paths.
    pub bins: BTreeMap<String, BinPath>,
    /// Named service endpoints.
    pub endpoints: BTreeMap<String, ServiceEndpoint>,
    /// Named references to provisioned secrets.
    pub secrets: BTreeMap<String, SecretRef>,
}

impl RuntimeManifest {
    /// Evaluate a manifest at an explicit path.
    ///
    /// Returns an error inside a Tokio runtime; async callers should use
    /// [`Self::load_from_async`] to avoid creating a nested runtime.
    pub fn load_from(path: &Path) -> Result<Self, LoadError> {
        load_pkl_sync(path).map_err(|source| LoadError {
            path: path.to_owned(),
            source,
        })
    }

    /// Evaluate a manifest using the caller's async runtime.
    pub async fn load_from_async(path: &Path) -> Result<Self, LoadError> {
        pklx::eval_to_typed(path, Default::default())
            .await
            .map_err(|source| LoadError {
                path: path.to_owned(),
                source,
            })
    }
}

pub(crate) fn load_pkl_sync<T: DeserializeOwned>(path: &Path) -> miette::Result<T> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err(miette::miette!(
            "load_sync cannot run inside a Tokio runtime; use load(path).await"
        ));
    }
    tokio::runtime::Runtime::new()
        .into_diagnostic()?
        .block_on(pklx::eval_to_typed(path, Default::default()))
}

/// An evaluator or deserialization failure with its source manifest path.
#[derive(Debug)]
pub struct LoadError {
    path: PathBuf,
    source: miette::Report,
}

impl LoadError {
    /// Path of the manifest that could not be loaded.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "evaluate {}: {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}
