//! Private host-to-supervisor policy. Producer settings are applied only after
//! namespace, filesystem, descriptor and privilege containment is established.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

pub const MAX_LOG_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Boundary {
    pub read_only_paths: Vec<PathBuf>,
    pub environment: BTreeMap<String, String>,
    pub sandbox: PathBuf,
}
