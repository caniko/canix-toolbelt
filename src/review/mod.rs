//! Provider-neutral, revision-bound pull-request review and merge gates.
//!
//! Consumers supply repository selection, credentials and policy. This module
//! preserves request intent and treats provider text as evidence, never commands.

mod model;
pub use model::*;
mod engine;
pub use engine::*;
mod github;
pub use github::roborev::{RoborevDocument, RoborevFinding, RoborevGitHub, RoborevReceipt};
pub use github::roborev_producer::{AuthorizedRoborevRequest, RoborevProgress};
pub use github::{GitHub, GreptileGitHub};
mod roborev_dispatch;
pub use roborev_dispatch::{
    RoborevDispatch, RoborevJobIdentity, RoborevRunner, dispatch_roborev_once,
};
#[cfg(unix)]
mod roborev_unix;
#[cfg(unix)]
pub use roborev_unix::{RoborevHttp, RoborevUnix, decode_roborev_http};
#[cfg(unix)]
mod roborev_local;
#[cfg(unix)]
pub use roborev_local::{
    RoborevLocal, RoborevLocalRequest, RoborevLocalScope, normalize_roborev_local,
};

/// Select exactly the configured provider/transport. Historical Greptile is
/// available only when explicitly selected; roborev never falls back to it.
pub fn github_provider(github: GitHub, policy: &Policy) -> Result<Box<dyn Provider>, Error> {
    policy.validate()?;
    match (policy.provider.as_str(), policy.transport.as_str()) {
        ("greptile", "github-comment") => Ok(Box::new(GreptileGitHub::new(github))),
        ("roborev", "github-receipt-v1") => {
            policy.policy_app_id()?;
            Ok(Box::new(RoborevGitHub::new(github)))
        }
        _ => Err(Error(
            "unsupported configured review provider/transport; no fallback is permitted".into(),
        )),
    }
}
#[cfg(feature = "cli")]
pub mod cli;
