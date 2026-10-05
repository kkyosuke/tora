mod cli;
mod update;

use clap::{CommandFactory, Parser};
use cli::{Cli, Commands};
use std::process::ExitCode;

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Commands::Aws { command } => aws::run(command),
        Commands::Update { version } => update::run(version.as_deref()),
        Commands::Completion { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "tora", &mut std::io::stdout());
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tora: {error}");
            ExitCode::FAILURE
        }
    }
}
mod aws;
