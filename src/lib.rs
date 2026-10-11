#![doc = include_str!("../RUST.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(all(unix, feature = "build-train"))]
pub mod build_train;
#[cfg(all(target_os = "linux", feature = "direct-network"))]
pub mod direct_network;
#[cfg(unix)]
pub mod operator;
#[cfg(all(unix, feature = "orchestration"))]
pub mod orchestration;
#[cfg(feature = "review")]
pub mod review;
pub mod runtime;
