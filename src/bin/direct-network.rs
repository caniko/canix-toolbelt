//! Root daemon and bounded readiness client for the direct-network module.
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Protect enrolled background cgroups from host VPN routing changes")]
struct Cli {
    #[command(subcommand)]
    command: Operation,
}

#[derive(Subcommand)]
enum Operation {
    /// Maintain direct routing and independent DNS from a root-owned policy.
    Daemon {
        #[arg(long)]
        policy: PathBuf,
    },
    /// Require current routing/DNS/cgroup readiness before launching a workload.
    Ready {
        #[arg(long, default_value = "/run/direct-network/ready.sock")]
        socket: PathBuf,
        /// Root-only startup barrier for the independent DNS listener itself.
        #[arg(long)]
        routes_only: bool,
    },
}

fn main() -> std::process::ExitCode {
    let result = match Cli::parse().command {
        Operation::Daemon { policy } => canix_toolbelt::direct_network::daemon(&policy),
        Operation::Ready {
            socket,
            routes_only,
        } => {
            if routes_only {
                canix_toolbelt::direct_network::routes_ready(&socket)
            } else {
                canix_toolbelt::direct_network::ready(&socket)
            }
        }
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("direct-network: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
