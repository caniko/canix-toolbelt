#![doc = include_str!("../RUST.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(all(unix, feature = "build-train"))]
pub mod build_train;
#[cfg(unix)]
pub mod operator;
#[cfg(feature = "review")]
pub mod review;
pub mod runtime;
