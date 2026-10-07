//! Compatibility paths for Fleetix's native construction service.
//!
//! Fleetix owns the native backend, immutable contracts, request roots,
//! coordinator and standalone frontend. Toolbelt preserves its public paths
//! through re-exports so consumers share those exact types and implementation.
//! Publication, resource admission and host activation remain caller-owned.

pub use fleetix::build_train::native::{Connection, Service, policy_identity, rollover, serve};

#[cfg(feature = "cli")]
/// Toolbelt's command path over the shared Fleetix frontend.
pub mod cli {
    pub use fleetix::build_train::cli::{Execution, TrainCommand, execute};

    /// Keep Toolbelt's historical frontend return type over Fleetix's runner.
    pub fn run(command: TrainCommand) -> Result<std::process::ExitCode, String> {
        fleetix::build_train::cli::run(command).map(|()| std::process::ExitCode::SUCCESS)
    }
}
