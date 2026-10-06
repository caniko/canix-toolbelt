//! Offline crash fixture; feature-gated out of production packages.
use anyhow::{Context, Result};
use canix_toolbelt_roborev_worker::execution::{ExecutionBinding, ExecutionFence};
use std::{
    io::{Read, Write},
    path::PathBuf,
};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let state = PathBuf::from(args.next().context("state directory required")?);
    let binding = PathBuf::from(args.next().context("binding file required")?);
    anyhow::ensure!(args.next().is_none(), "unexpected fixture argument");
    let binding: ExecutionBinding = serde_json::from_slice(&std::fs::read(binding)?)?;
    let fence = ExecutionFence::load(&state, &binding)?;
    let reservation = fence.reserve()?;
    if std::env::var_os("CANIX_EXECUTION_FIXTURE_NO_READY").is_none() {
        println!("reserved");
    }
    std::io::stdout().flush()?;
    std::io::stdin().read_exact(&mut [0u8; 1])?;
    // Keep the real kernel lock until the parent kills this disposable fixture.
    drop(reservation);
    Ok(())
}
