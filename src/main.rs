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
    /// Execute source-bound qualification and verify portable evidence
    #[cfg(feature = "qualification")]
    #[command(subcommand)]
    Qualification(canix_toolbelt::qualification::cli::QualificationCommand),
    /// Run or inspect the builder-local shared construction service
    #[cfg(all(unix, feature = "build-train"))]
    #[command(subcommand)]
    BuildTrain(canix_toolbelt::build_train::cli::TrainCommand),
    /// Ensure revision-bound PR review, inspect gates and disposition findings
    #[command(subcommand)]
    Review(canix_toolbelt::review::cli::ReviewCommand),
    /// Revalidate review and CI before an explicitly authorized PR merge
    Merge(canix_toolbelt::review::cli::MergeArgs),
    /// Inspect deployment-provided runtime facts
    #[command(subcommand)]
    Runtime(RuntimeCommand),
    /// Bump Caddy plugins and refresh the flake's vendor `pluginsHash`
    CaddyPlugins {
        /// Flake checkout exposing `legacyPackages.<system>.caddyPluginsVendorFor`
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Keep plugin versions; only recompute the hash
        #[arg(long)]
        hash_only: bool,
        /// Report available updates without editing or building
        #[arg(long)]
        dry_run: bool,
        /// Command run with the realized vendor store path appended
        /// (for example `-- attic push CACHE`)
        #[arg(last = true)]
        publish: Vec<String>,
    },
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
        #[cfg(feature = "qualification")]
        Command::Qualification(command) => {
            let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
                signal_hook::flag::register(signal, std::sync::Arc::clone(&cancelled))?;
            }
            let result = canix_toolbelt::qualification::cli::dispatch(command, &cancelled)?;
            serde_json::to_writer_pretty(std::io::stdout().lock(), &result.value)?;
            println!();
            return Ok(std::process::ExitCode::from(if result.passed {
                0
            } else {
                1
            }));
        }
        #[cfg(all(unix, feature = "build-train"))]
        Command::BuildTrain(command) => {
            return canix_toolbelt::build_train::cli::run(command).map_err(Into::into);
        }
        Command::Review(command) => {
            return canix_toolbelt::review::cli::run(command)
                .map(std::process::ExitCode::from)
                .map_err(Into::into);
        }
        Command::Merge(args) => {
            return canix_toolbelt::review::cli::merge(args)
                .map(std::process::ExitCode::from)
                .map_err(Into::into);
        }
        Command::Runtime(RuntimeCommand::Show { path }) => {
            let manifest = RuntimeManifest::load_from(&path)?;
            let mut stdout = std::io::stdout().lock();
            serde_json::to_writer_pretty(&mut stdout, &manifest)?;
            writeln!(stdout)?;
        }
        Command::CaddyPlugins {
            root,
            hash_only,
            dry_run,
            publish,
        } => {
            use canix_toolbelt::caddy_plugins;
            let refresh =
                caddy_plugins::refresh(&root, &mut caddy_plugins::NixCommand, hash_only, dry_run)
                    .map_err(|error| error as Box<dyn std::error::Error>)?;
            if let (Some(resolved), Some((program, args))) =
                (&refresh.resolved, publish.split_first())
            {
                let status = std::process::Command::new(program)
                    .args(args)
                    .arg(&resolved.store_path)
                    .status()?;
                if !status.success() {
                    return Err(format!("publish command failed: {status}").into());
                }
            }
            let report = serde_json::json!({
                "plugins": refresh.plugins.iter().map(|p| format!("{}@{}", p.module, p.version)).collect::<Vec<_>>(),
                "updates": refresh.updates.iter().map(|(module, old, new)| serde_json::json!({"module": module, "from": old, "to": new})).collect::<Vec<_>>(),
                "hash": refresh.resolved.as_ref().map(|r| &r.hash),
                "storePath": refresh.resolved.as_ref().map(|r| &r.store_path),
            });
            let mut stdout = std::io::stdout().lock();
            serde_json::to_writer_pretty(&mut stdout, &report)?;
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
                        operator::Outcome::Succeeded | operator::Outcome::Cancelled => 0,
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
