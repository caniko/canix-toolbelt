//! Root daemon and bounded readiness client for the direct-network module.
#[cfg(target_os = "linux")]
mod linux {
    use clap::{Parser, Subcommand};
    use std::ffi::OsString;
    use std::os::unix::process::CommandExt;
    use std::path::PathBuf;
    use std::process::Command;

    #[derive(Parser)]
    #[command(about = "Protect enrolled background cgroups from host VPN routing changes")]
    struct Cli {
        #[command(subcommand)]
        command: Operation,
    }

    #[derive(Subcommand)]
    enum Operation {
        /// Keep a dedicated enrolled slice alive without polling or performing network work.
        Anchor,
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
        /// Start the selected user anchors, require protection, then replace this process with a desktop VPN client.
        Launch {
            #[arg(long, default_value = "/run/direct-network/ready.sock")]
            socket: PathBuf,
            #[arg(long)]
            systemctl: PathBuf,
            #[arg(long)]
            unit: Vec<String>,
            #[arg(required = true, trailing_var_arg = true)]
            command: Vec<OsString>,
        },
    }

    pub(super) fn run() -> std::process::ExitCode {
        let result = match Cli::parse().command {
            Operation::Anchor => loop {
                std::thread::park();
            },
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
            Operation::Launch {
                socket,
                systemctl,
                unit,
                command,
            } => launch(&socket, &systemctl, &unit, &command),
        };
        match result {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("direct-network: {error}");
                std::process::ExitCode::FAILURE
            }
        }
    }

    fn launch(
        socket: &std::path::Path,
        systemctl: &std::path::Path,
        units: &[String],
        command: &[OsString],
    ) -> canix_toolbelt::direct_network::Result<()> {
        if !units.is_empty() {
            let status = Command::new(systemctl)
                .args(["--user", "start"])
                .args(units)
                .status()?;
            if !status.success() {
                return Err(std::io::Error::other(format!(
                    "direct-network user anchors failed: {status}"
                ))
                .into());
            }
        }
        canix_toolbelt::direct_network::ready(socket)?;
        // Clap requires at least one command argument.
        Err(Command::new(&command[0]).args(&command[1..]).exec().into())
    }
}

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    linux::run()
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("direct-network requires Linux cgroup v2 and nftables");
    std::process::ExitCode::FAILURE
}
