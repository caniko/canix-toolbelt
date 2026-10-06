use super::Limits;
use anyhow::{Result, ensure};
use std::time::{Duration, Instant};

/// Cumulative helper and reaped-descendant CPU, plus one monotonic deadline.
pub(super) struct Budget {
    deadline: Instant,
    cpu_start: Duration,
    cpu_limit: Duration,
}

fn cpu_used() -> Result<Duration> {
    let mut total = Duration::ZERO;
    use nix::sys::resource::{UsageWho, getrusage};
    for who in [UsageWho::RUSAGE_SELF, UsageWho::RUSAGE_CHILDREN] {
        let usage = getrusage(who)?;
        for time in [usage.user_time(), usage.system_time()] {
            total += Duration::from_secs(time.tv_sec() as u64)
                + Duration::from_micros(time.tv_usec() as u64);
        }
    }
    Ok(total)
}

impl Budget {
    pub(super) fn new(limits: &Limits) -> Result<Self> {
        Ok(Self {
            deadline: Instant::now() + Duration::from_secs(limits.wall_seconds),
            cpu_start: cpu_used()?,
            cpu_limit: Duration::from_secs(limits.cpu_seconds),
        })
    }

    pub(super) fn check(&self) -> Result<()> {
        ensure!(
            Instant::now() < self.deadline,
            "whole preparation exceeded its time bound"
        );
        ensure!(
            cpu_used()?.saturating_sub(self.cpu_start) < self.cpu_limit,
            "whole preparation exceeded its cumulative CPU bound"
        );
        Ok(())
    }

    pub(super) fn remaining_cpu_seconds(&self) -> Result<u64> {
        self.check()?;
        let remaining = self
            .cpu_limit
            .saturating_sub(cpu_used()?.saturating_sub(self.cpu_start));
        // Kernel CPU rlimits use integral seconds. Check the precise cumulative
        // budget after each child and during Rust copy/hash work.
        Ok(remaining.as_secs() + u64::from(remaining.subsec_nanos() != 0))
    }
}
