use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, arg_required_else_help = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Install the latest stable release (or a specific version)
    Update {
        /// Release version, e.g. v0.1.0 (allows reinstall and downgrade)
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
    },
    /// Generate a shell completion script
    Completion {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_tree_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn update_version_is_explicit() {
        assert!(matches!(
            Cli::try_parse_from(["tora", "update", "--version", "v0.2.0"]).unwrap().command,
            Commands::Update { version: Some(v) } if v == "v0.2.0"
        ));
        assert!(Cli::try_parse_from(["tora", "update", "--unknown"]).is_err());
    }
}
