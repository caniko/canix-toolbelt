//! Minimal wire contract shared with the standalone PID-1 supervisor.

/// Private native supervisor output, retained independently of launcher exit status.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionReport {
    /// Original producer exit code, absent for signal termination.
    pub exit_code: Option<i32>,
    /// Original producer termination signal, absent for ordinary exits.
    pub signal: Option<i32>,
    /// Kernel child accounting confirmed ECHILD after reaping the leader.
    pub descendants_drained: bool,
    /// Identity of bytes, names, types and modes retained from the private work tree.
    pub work_sha256: Option<String>,
}
