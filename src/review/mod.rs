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
#[cfg(feature = "cli")]
pub mod cli;
