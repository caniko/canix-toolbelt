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
    /// Ensure revision-bound PR review, inspect gates and disposition findings
    #[command(subcommand)]
    Review(canix_toolbelt::review::cli::ReviewCommand),
    /// Revalidate review and CI before an explicitly authorized PR merge
    Merge(canix_toolbelt::review::cli::MergeArgs),
    /// Inspect deployment-provided runtime facts
    #[command(subcommand)]
    Runtime(RuntimeCommand),
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
