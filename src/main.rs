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

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Runtime(RuntimeCommand::Show { path }) => {
            let manifest = RuntimeManifest::load_from(&path)?;
            let mut stdout = std::io::stdout().lock();
            serde_json::to_writer_pretty(&mut stdout, &manifest)?;
            writeln!(stdout)?;
        }
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run(Cli::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
