mod cli;
mod completion;
mod config;
mod doctor;
mod shell;
mod update;

use clap::Parser;
use cli::{Cli, Commands};
use std::process::ExitCode;

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Commands::Aws { command } => aws::run(command),
        Commands::Update { version, check } => {
            if check {
                update::check()
            } else {
                update::run(version.as_deref())
            }
        }
        Commands::Doctor { profile } => doctor::run(profile),
        Commands::Config { command } => config::run(command),
        Commands::Completion { command } => completion::run(command),
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
