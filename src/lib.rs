#![doc = include_str!("../RUST.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod execution_protocol;

#[cfg(feature = "qualification")]
pub mod qualification;

#[cfg(all(unix, feature = "build-train"))]
pub mod build_train;
#[cfg(feature = "caddy-plugins")]
pub mod caddy_plugins;
#[cfg(all(target_os = "linux", feature = "direct-network"))]
pub mod direct_network;
#[cfg(unix)]
pub mod operator;
#[cfg(all(unix, feature = "orchestration"))]
pub mod orchestration;
#[cfg(feature = "review")]
pub mod review;
pub mod runtime;
