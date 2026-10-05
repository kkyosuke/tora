use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, arg_required_else_help = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// AWS profile selection and SSO helpers (requires AWS CLI v2)
    Aws {
        #[command(subcommand)]
        command: AwsCommands,
    },
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

#[derive(Subcommand)]
pub enum AwsCommands {
    /// List configured profiles using AWS CLI's configuration resolution
    Profiles,
    /// Log in through IAM Identity Center and open a shell with the selected profile
    Sso {
        /// Profile name; omit to select interactively
        profile: Option<String>,
        /// Print a Bash/Zsh export statement after successful login
        #[arg(long = "export", conflicts_with = "no_shell")]
        export: bool,
        /// Log in without opening an authenticated shell
        #[arg(long)]
        no_shell: bool,
        /// Do not automatically open the browser
        #[arg(long)]
        no_browser: bool,
        /// Use device-code authorization for login from another device
        #[arg(long)]
        use_device_code: bool,
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
