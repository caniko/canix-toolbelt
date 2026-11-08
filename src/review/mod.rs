//! Provider-neutral, revision-bound pull-request review and merge gates.
//!
//! Consumers supply repository selection, credentials and policy. This module
//! preserves request intent and treats provider text as evidence, never commands.

mod model;
pub use model::*;
mod engine;
pub use engine::*;
mod github;
pub use github::{GitHub, GreptileGitHub};

/// Select exactly the configured provider/transport. Historical Greptile is
/// available only when explicitly selected.
pub fn github_provider(github: GitHub, policy: &Policy) -> Result<Box<dyn Provider>, Error> {
    policy.validate()?;
    match (policy.provider.as_str(), policy.transport.as_str()) {
        ("greptile", "github-comment") => Ok(Box::new(GreptileGitHub::new(github))),
        _ => Err(Error(
            "unsupported configured review provider/transport; no fallback is permitted".into(),
        )),
    }
}
#[cfg(feature = "cli")]
pub mod cli;
