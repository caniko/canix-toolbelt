use canix_toolbelt::runtime::RuntimeManifest;
use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect deployment-provided runtime facts
    #[command(subcommand)]
    Runtime(RuntimeCommand),
    /// Run or cancel a durable systemd stage controller
    #[cfg(unix)]
    #[command(subcommand)]
    Operator(OperatorCommand),
}

#[cfg(unix)]
#[derive(Subcommand)]
enum OperatorCommand {
    /// Execute or resume the immutable deployment policy
    Run {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        systemctl: PathBuf,
        #[arg(long)]
        systemd_notify: Option<PathBuf>,
    },
    /// Cancel after the controller service has stopped
    Cancel {
        #[arg(long)]
        config: PathBuf,
    },
}

#[derive(Subcommand)]
enum RuntimeCommand {
    /// Evaluate a Pkl runtime manifest and print its typed JSON representation
    Show {
        /// Consumer-owned runtime manifest
        #[arg(long)]
        path: PathBuf,
    },
}

fn run(cli: Cli) -> Result<std::process::ExitCode, Box<dyn std::error::Error>> {
    match cli.command {
        Command::Runtime(RuntimeCommand::Show { path }) => {
            let manifest = RuntimeManifest::load_from(&path)?;
            let mut stdout = std::io::stdout().lock();
            serde_json::to_writer_pretty(&mut stdout, &manifest)?;
            writeln!(stdout)?;
        }
        #[cfg(unix)]
        Command::Operator(command) => {
            use canix_toolbelt::operator;
            match command {
                OperatorCommand::Cancel { config } => {
                    let policy = serde_json::from_reader(std::fs::File::open(config)?)?;
                    operator::cancel(&policy)?;
                }
                OperatorCommand::Run {
                    config,
                    systemctl,
                    systemd_notify,
                } => {
                    let policy = serde_json::from_reader(std::fs::File::open(config)?)?;
                    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
                        signal_hook::flag::register(signal, std::sync::Arc::clone(&cancelled))?;
                    }
                    let outcome = operator::run_with_ready(
                        &policy,
                        &mut operator::Systemd { systemctl },
                        &cancelled,
                        || {
                            if let Some(notify) = systemd_notify {
                                if !std::process::Command::new(notify)
                                    .arg("--ready")
                                    .status()?
                                    .success()
                                {
                                    return Err(std::io::Error::other(
                                        "systemd readiness notification failed",
                                    ));
                                }
                            }
                            Ok(())
                        },
                    )?;
                    return Ok(std::process::ExitCode::from(match outcome {
                        operator::Outcome::Succeeded => 0,
                        operator::Outcome::Failed => 20,
                        operator::Outcome::Interrupted => 143,
                    }));
                }
            }
        }
    }
    Ok(std::process::ExitCode::SUCCESS)
}

fn main() -> std::process::ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
