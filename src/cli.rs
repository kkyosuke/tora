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
    /// Check for updates or install a stable release
    Update {
        /// Release version, e.g. v0.1.0 (allows reinstall and downgrade)
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
        /// Compare versions without installing (also works with source builds)
        #[arg(long, conflicts_with = "version")]
        check: bool,
    },
    /// Diagnose installation, PATH, completion and AWS authentication
    Doctor {
        /// Check this AWS profile instead of the current credential environment
        #[arg(long)]
        profile: Option<String>,
    },
    /// Show or change tora defaults (does not modify AWS configuration files)
    Config {
        #[command(subcommand)]
        command: Option<ConfigCommand>,
    },
    /// Generate or install shell completion
    Completion {
        #[command(subcommand)]
        command: CompletionCommand,
    },
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Show all saved defaults and the configuration path
    List,
    /// Print a saved value (fails if unset)
    Get { key: ConfigKey },
    /// Save a default
    Set { key: ConfigKey, value: String },
    /// Remove a default
    Unset { key: ConfigKey },
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum ConfigKey {
    #[value(name = "aws.profile")]
    Profile,
    #[value(name = "aws.region")]
    Region,
}

#[derive(Subcommand)]
pub enum CompletionCommand {
    /// Generate Bash completion
    Bash,
    /// Generate Zsh completion
    Zsh,
    /// Generate Fish completion
    Fish,
    /// Generate PowerShell completion
    Powershell,
    /// Generate Elvish completion
    Elvish,
    /// Install completion and its shell startup configuration
    Install {
        /// Override shell detection
        #[arg(long, value_enum)]
        shell: Option<InstallShell>,
        /// Completion loader file (default: $TORA_HOME/completions/<shell>)
        #[arg(long)]
        path: Option<std::path::PathBuf>,
        /// Accept the displayed changes without prompting (requires --shell)
        #[arg(long, requires = "shell")]
        yes: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum InstallShell {
    Bash,
    Zsh,
}

#[derive(Subcommand)]
pub enum AwsCommands {
    /// Show account, ARN, user ID and region without logging in
    Whoami {
        /// Inspect a specific profile instead of current credentials
        profile: Option<String>,
    },
    /// Log out ALL cached AWS SSO sessions across all profiles
    Logout,
    /// Run a command with a profile's existing credentials (does not log in)
    Exec {
        /// Profile name; defaults to environment/config, otherwise an interactive menu
        profile: Option<String>,
        /// Command and arguments, after -- (executed without a shell)
        #[arg(last = true, required = true, num_args = 1..)]
        command: Vec<std::ffi::OsString>,
    },
    /// List configured profiles using AWS CLI's configuration resolution
    Profiles,
    /// Log in through IAM Identity Center and open a shell with the selected profile
    Sso {
        /// Profile name; defaults to environment/config, otherwise a menu
        profile: Option<String>,
        /// Select a profile interactively, ignoring environment and saved defaults
        #[arg(long, conflicts_with = "profile")]
        select: bool,
        /// Print a Bash/Zsh export statement after authentication succeeds
        #[arg(long = "export", conflicts_with = "no_shell")]
        export: bool,
        /// Ensure authentication without opening a shell
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
            Commands::Update { version: Some(v), check: false } if v == "v0.2.0"
        ));
        assert!(Cli::try_parse_from(["tora", "update", "--unknown"]).is_err());
    }
}
