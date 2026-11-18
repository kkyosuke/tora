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
    /// Generate ~/.aws/config profiles for every SSO account and role (shows changes first)
    Sync {
        /// sso-session section to use; prompts when several exist
        #[arg(long, value_name = "NAME")]
        sso_session: Option<String>,
        /// Region for generated profiles (default: the session's sso_region)
        #[arg(long)]
        region: Option<String>,
        /// Print the generated managed block without changing files
        #[arg(long, conflicts_with = "yes")]
        dry_run: bool,
        /// Apply the displayed changes without prompting (required without a terminal)
        #[arg(long)]
        yes: bool,
        /// Do not automatically open the browser when SSO login is needed
        #[arg(long)]
        no_browser: bool,
        /// Use device-code authorization when SSO login is needed
        #[arg(long)]
        use_device_code: bool,
    },
    /// Run a command with a profile's credentials (logs in only with --login)
    Exec {
        /// Profile name; defaults to environment/config, otherwise an interactive menu
        profile: Option<String>,
        /// Check authentication first and log in through SSO only when needed
        #[arg(long)]
        login: bool,
        /// Do not automatically open the browser
        #[arg(long, requires = "login")]
        no_browser: bool,
        /// Use device-code authorization for login from another device
        #[arg(long, requires = "login")]
        use_device_code: bool,
        /// Command and arguments, after -- (executed without a shell)
        #[arg(last = true, required = true, num_args = 1..)]
        command: Vec<std::ffi::OsString>,
    },
    /// Print the current profile and SSO time left for a shell prompt (fast, never fails)
    #[command(
        long_about = "Print the current profile and SSO time left for a shell prompt.\n\n\
            Uses AWS_PROFILE or AWS_DEFAULT_PROFILE (not the saved aws.profile) and reads \
            the AWS config file and SSO token cache directly, without AWS CLI or the network. \
            Prints nothing when no profile is set and never writes to stderr or fails.\n\n\
            Time left is like 52m or 3h 13m, or `expired` once the SSO token has expired \
            (as in `tora aws status`)."
    )]
    Prompt {
        /// Output template with {profile} and {remaining} (empty when unknown)
        #[arg(long, value_name = "TEMPLATE")]
        format: Option<String>,
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
    /// Start a Session Manager session on an instance (requires session-manager-plugin)
    Ssm {
        /// Profile name; defaults to environment/config, otherwise a menu
        profile: Option<String>,
        /// Instance or managed node ID (i-… / mi-…); omit to select from a list
        #[arg(long, short = 't', value_name = "ID")]
        target: Option<String>,
        /// Select a profile interactively, ignoring environment and saved defaults
        #[arg(long, conflicts_with = "profile")]
        select: bool,
        /// Do not automatically open the browser when SSO login is needed
        #[arg(long)]
        no_browser: bool,
        /// Use device-code authorization when SSO login is needed
        #[arg(long)]
        use_device_code: bool,
    },
    /// Forward a local port to a remote host through an instance (requires session-manager-plugin)
    Tunnel {
        /// Profile name; defaults to environment/config, otherwise a menu
        profile: Option<String>,
        /// Instance to forward through (i-… / mi-…); omit to select from a list
        #[arg(long, short = 't', value_name = "ID")]
        target: Option<String>,
        /// Remote host and port, e.g. mydb.example.com:5432; omit to select from history
        #[arg(long, short = 'r', value_name = "HOST:PORT")]
        remote: Option<String>,
        /// Local port (default: the remote port, or a free port if it is in use)
        #[arg(long, short = 'l', value_name = "PORT", value_parser = clap::value_parser!(u16).range(1..))]
        local: Option<u16>,
        /// Select a profile interactively, ignoring environment and saved defaults
        #[arg(long, conflicts_with = "profile")]
        select: bool,
        /// Do not automatically open the browser when SSO login is needed
        #[arg(long)]
        no_browser: bool,
        /// Use device-code authorization when SSO login is needed
        #[arg(long)]
        use_device_code: bool,
    },
    /// Show whoami for a profile plus its SSO token expiry (does not log in)
    Status {
        /// Profile name; defaults to AWS_PROFILE, AWS_DEFAULT_PROFILE or aws.profile, otherwise a menu
        profile: Option<String>,
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
