mod history;
mod settings;
mod ssm;
mod status;
mod sync;

use settings::Settings;

use crate::cli::AwsCommands;
use dialoguer::{Input, Select, theme::ColorfulTheme};
use std::{
    collections::HashSet,
    io::{self, BufRead, IsTerminal, Write},
    process::{Command, Output, Stdio},
};

pub fn run(command: AwsCommands) -> io::Result<()> {
    let settings = Settings::load()?;
    match command {
        AwsCommands::Whoami { profile } => {
            print!("{}", identity_with_settings(profile, &settings)?);
            Ok(())
        }
        AwsCommands::Logout => {
            eprintln!("Logging out ALL cached AWS SSO sessions across all profiles.");
            let status = settings
                .command()
                .args(["sso", "logout"])
                .status()
                .map_err(aws_error)?;
            if !status.success() {
                return Err(io::Error::other(format!("aws sso logout failed: {status}")));
            }
            println!("Logged out all cached SSO sessions. Parent shell environment is unchanged.");
            Ok(())
        }
        AwsCommands::Exec {
            profile,
            login,
            no_browser,
            use_device_code,
            command,
        } => {
            let profile = resolve_profile(profile.or(settings.profile.clone()), &settings)?;
            if login {
                ensure_login(&profile, &settings, no_browser, use_device_code)?;
            }
            let mut child = Command::new(&command[0]);
            child.args(&command[1..]);
            settings.configure_profile(&mut child, &profile);
            history::remember(&profile);
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                Err(child.exec())
            }
            #[cfg(not(unix))]
            {
                let status = child.status()?;
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        AwsCommands::Profiles => {
            let profiles = profiles(&settings)?;
            let mut out = io::stdout().lock();
            for profile in profiles {
                writeln!(out, "{profile}")?;
            }
            Ok(())
        }
        AwsCommands::Sso {
            profile,
            select,
            export,
            no_shell,
            no_browser,
            use_device_code,
        } => {
            let profile = if select {
                None
            } else {
                profile.or(settings.profile.clone())
            };
            let profile = resolve_profile(profile, &settings)?;
            ensure_login(&profile, &settings, no_browser, use_device_code)?;
            history::remember(&profile);
            if export {
                writeln!(
                    io::stdout().lock(),
                    "{}",
                    settings.export_statement(&profile)
                )?;
            } else if no_shell {
                eprintln!("To use this profile in your current Bash/Zsh shell:");
                eprintln!("  {}", settings.export_statement(&profile));
            } else {
                return open_shell(&profile, &settings);
            }
            Ok(())
        }
        AwsCommands::Ssm {
            profile,
            target,
            select,
            no_browser,
            use_device_code,
        } => {
            // Only the positional argument can be a mistyped instance ID; environment and
            // saved defaults are real profile names.
            ssm::check_profile_argument(profile.as_deref())?;
            let profile = if select {
                None
            } else {
                profile.or(settings.profile.clone())
            };
            ssm::run(profile, target, &settings, no_browser, use_device_code)
        }
        AwsCommands::Sync {
            sso_session,
            region,
            dry_run,
            yes,
            no_browser,
            use_device_code,
        } => sync::run(
            sync::Options {
                sso_session,
                region,
                dry_run,
                yes,
                no_browser,
                use_device_code,
            },
            &settings,
        ),
        AwsCommands::Status { profile } => {
            let profile = resolve_profile(profile.or(settings.profile.clone()), &settings)?;
            status::run(&profile, &settings)
        }
    }
}

fn credentials_valid(profile: &str, settings: &Settings) -> io::Result<bool> {
    let output = settings
        .profile_command(profile)
        .args(["sts", "get-caller-identity", "--profile", profile])
        .stdout(Stdio::null())
        .output()
        .map_err(aws_error)?;
    authenticated(profile, &output)
}

// Ok(false) means a login is needed; unrelated failures are reported as errors.
fn authenticated(profile: &str, output: &Output) -> io::Result<bool> {
    if output.status.success() {
        return Ok(true);
    }
    let error = String::from_utf8_lossy(&output.stderr);
    // AWS CLI does not expose structured errors for local SSO cache failures.
    // Only known missing/expired authentication errors should trigger a browser.
    let message = error.to_ascii_lowercase();
    if message.contains("token has expired and refresh failed")
        || message.contains(
            "sso session associated with this profile has expired or is otherwise invalid",
        )
        || (message.contains("error loading sso token") && message.contains("does not exist"))
        || message.contains("(expiredtoken)")
        || message.contains("(expiredtokenexception)")
        || message.contains("(invalidgrantexception)")
        || (message.contains("(unauthorizedexception)") && message.contains("getrolecredentials"))
    {
        return Ok(false);
    }
    Err(io::Error::other(format!(
        "AWS authentication check failed for profile {profile:?}: {}\n{}",
        output.status,
        error.trim()
    )))
}

// Reuse valid credentials and log in only for known missing/expired authentication.
// All messages and login output go to stderr so stdout stays safe to eval or pipe.
fn ensure_login(
    profile: &str,
    settings: &Settings,
    no_browser: bool,
    use_device_code: bool,
) -> io::Result<()> {
    if credentials_valid(profile, settings)? {
        eprintln!("AWS credentials are valid for profile {profile:?}; skipping login.");
        return Ok(());
    }
    sso_login(
        settings.profile_command(profile),
        &["--profile", profile],
        &format!("profile {profile:?}"),
        no_browser,
        use_device_code,
    )
}

/// Runs `aws sso login` for `target` (`--profile NAME` or `--sso-session NAME`).
fn sso_login(
    mut command: Command,
    target: &[&str],
    label: &str,
    no_browser: bool,
    use_device_code: bool,
) -> io::Result<()> {
    eprintln!("Logging in with AWS {label}...");
    command.args(["sso", "login"]).args(target);
    if no_browser {
        command.arg("--no-browser");
    }
    if use_device_code {
        command.arg("--use-device-code");
    }
    // Preserve browser/device login interaction while keeping stdout safe to eval.
    let status = command
        .stdout(Stdio::from(io::stderr()))
        .status()
        .map_err(aws_error)?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "aws sso login failed for {label}: {status}"
        )));
    }
    eprintln!("SSO login completed for {label}.");
    Ok(())
}

fn open_shell(profile: &str, settings: &Settings) -> io::Result<()> {
    let shell = std::env::var_os("SHELL")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    let mut command = Command::new(&shell);
    let _startup = crate::shell::configure(&mut command, &shell)?;
    command.arg("-i");
    settings.configure_profile(&mut command, profile);
    eprintln!("Opening shell with AWS_PROFILE={profile:?}. Type exit to return.");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Replace tora so terminal signals, job control and exit status belong to the shell.
        Err(command.exec())
    }
    #[cfg(not(unix))]
    {
        let status = command.status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("shell failed: {status}")))
        }
    }
}

fn resolve_profile(profile: Option<String>, settings: &Settings) -> io::Result<String> {
    let profile = match profile {
        Some(profile) => profile,
        None => {
            let mut profiles = profiles(settings)?;
            let recent = history::recent_profiles();
            history::sort_by_recent(&mut profiles, &recent, |p| p);
            if io::stdin().is_terminal() && io::stderr().is_terminal() {
                select_terminal_profile(&profiles, &recent)?
            } else {
                select_profile(
                    &profiles,
                    &recent,
                    &mut io::stdin().lock(),
                    &mut io::stderr().lock(),
                )?
            }
        }
    };
    validate_profile(&profile)?;
    Ok(profile)
}

fn filtered_profiles(profiles: &[String], query: &str) -> Vec<String> {
    let query = query.to_lowercase();
    profiles
        .iter()
        .filter(|p| p.to_lowercase().contains(&query))
        .cloned()
        .collect()
}

fn select_terminal_profile(profiles: &[String], recent: &[String]) -> io::Result<String> {
    let theme = ColorfulTheme::default();
    let mut visible = profiles.to_vec();
    while !profiles.is_empty() {
        let mut items: Vec<String> = visible.iter().map(|p| profile_label(p, recent)).collect();
        items.push("Type a custom profile".into());
        items.push("Search profiles".into());
        let selection = Select::with_theme(&theme)
            .with_prompt("AWS profile (Esc/q to cancel)")
            .items(&items)
            .default(0)
            .interact_opt()
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("profile selection cancelled"))?;
        if selection < visible.len() {
            return Ok(visible[selection].clone());
        }
        if selection == visible.len() {
            break;
        }
        let query: String = Input::with_theme(&theme)
            .with_prompt("Search profile name (empty to show all)")
            .allow_empty(true)
            .interact_text()
            .map_err(io::Error::other)?;
        visible = filtered_profiles(profiles, &query);
        if visible.is_empty() {
            eprintln!("No matching profiles. Search again or enter a custom profile.");
        }
    }
    Input::<String>::with_theme(&theme)
        .with_prompt("AWS profile name")
        .validate_with(|profile: &String| validate_profile(profile))
        .interact_text()
        .map_err(io::Error::other)
}

fn aws_error(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::NotFound {
        io::Error::new(
            error.kind(),
            "aws CLI not found in PATH; install/configure AWS CLI v2 first",
        )
    } else {
        error
    }
}

/// Profile names for shell completion. Listing profiles does not depend on
/// saved defaults, so a broken tora config cannot hide candidates.
pub(crate) fn completion_profiles() -> io::Result<Vec<String>> {
    profiles(&Settings::default())
}

fn profiles(settings: &Settings) -> io::Result<Vec<String>> {
    // AWS CLI handles config, credentials, default, sso-session sections and custom paths.
    let output = settings
        .command()
        .args(["configure", "list-profiles"])
        .stderr(Stdio::inherit())
        .output()
        .map_err(aws_error)?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "aws configure list-profiles failed: {}",
            output.status
        )));
    }
    let output = String::from_utf8(output.stdout)
        .map_err(|_| io::Error::other("AWS profile names must be valid UTF-8"))?;
    let mut seen = HashSet::new();
    let mut profiles = Vec::new();
    for profile in output.lines().filter(|line| !line.is_empty()) {
        validate_profile(profile)?;
        if seen.insert(profile) {
            profiles.push(profile.to_owned());
        }
    }
    Ok(profiles)
}

fn validate_profile(profile: &str) -> io::Result<()> {
    if profile.trim().is_empty() || profile.chars().any(char::is_control) {
        return Err(io::Error::other(
            "profile name must be non-empty and contain no control characters",
        ));
    }
    Ok(())
}

fn read_line(input: &mut impl BufRead) -> io::Result<String> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Err(io::Error::other(
            "no profile selected (end of input); pass a profile argument for non-interactive use",
        ));
    }
    Ok(line.trim().to_owned())
}

fn profile_label(profile: &str, recent: &[String]) -> String {
    format!("{profile}{}", history::marker(profile, recent))
}

fn select_profile(
    profiles: &[String],
    recent: &[String],
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<String> {
    let mut visible = profiles.to_vec();
    loop {
        if profiles.is_empty() {
            writeln!(
                output,
                "No AWS profiles found. Enter a custom profile name."
            )?;
        } else {
            writeln!(output, "Select AWS profile:")?;
            for (index, profile) in visible.iter().enumerate() {
                writeln!(
                    output,
                    "  {}. {}",
                    index + 1,
                    profile_label(profile, recent)
                )?;
            }
            writeln!(output, "  0. Type a custom profile")?;
        }
        if !profiles.is_empty() {
            write!(
                output,
                "Enter number (0 for custom, /text to search, / to reset, q to cancel): "
            )?;
            output.flush()?;
            let selection = read_line(input)?;
            if selection == "q" {
                return Err(io::Error::other("profile selection cancelled"));
            }
            if let Some(query) = selection.strip_prefix('/') {
                visible = filtered_profiles(profiles, query);
                if visible.is_empty() {
                    writeln!(output, "No matching profiles.")?;
                }
                continue;
            }
            match selection.parse::<usize>() {
                Ok(0) => {}
                Ok(index) if index <= visible.len() => return Ok(visible[index - 1].clone()),
                _ => {
                    writeln!(output, "Invalid selection. Try again.")?;
                    continue;
                }
            }
        }
        write!(output, "Profile name: ")?;
        output.flush()?;
        let profile = read_line(input)?;
        match validate_profile(&profile) {
            Ok(()) => return Ok(profile),
            Err(error) => writeln!(output, "{error}. Try again.")?,
        }
    }
}

/// SSO token expiry of the explicit or default profile, for doctor.
/// Ok(None) when no profile is selected; an error when the token has expired.
pub fn sso_expiry(explicit: Option<String>) -> io::Result<Option<String>> {
    let settings = Settings::load()?;
    let Some(profile) = explicit.or(settings.profile.clone()) else {
        return Ok(None);
    };
    validate_profile(&profile)?;
    status::expiry(&profile, &settings).map(Some)
}

// With no explicit argument, preserve ambient credentials: whoami must describe
// the actual identity, including environment credentials that override profiles.
pub fn identity(explicit: Option<String>) -> io::Result<String> {
    identity_with_settings(explicit, &Settings::load()?)
}

fn identity_with_settings(explicit: Option<String>, settings: &Settings) -> io::Result<String> {
    let profile = explicit.clone().or(settings.profile.clone());
    if let Some(profile) = &profile {
        validate_profile(profile)?;
    }
    let make_command = || {
        if let Some(profile) = &explicit {
            explicit_command(profile, settings)
        } else {
            let mut command = settings.command();
            if let Some(profile) = &profile {
                command.env("AWS_PROFILE", profile);
                command.env_remove("AWS_DEFAULT_PROFILE");
            }
            command
        }
    };
    let output = caller_identity(make_command())?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "AWS identity check failed: {}\n{}\nRun tora aws sso for missing or expired SSO credentials.",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    describe_identity(
        profile.as_deref().unwrap_or("default (credential chain)"),
        &output,
        make_command,
        settings,
    )
}

/// An AWS CLI command for an explicitly selected profile, ignoring ambient credentials.
fn explicit_command(profile: &str, settings: &Settings) -> Command {
    let mut command = settings.profile_command(profile);
    command.args(["--profile", profile]);
    command
}

fn caller_identity(mut command: Command) -> io::Result<Output> {
    command
        .args([
            "sts",
            "get-caller-identity",
            "--query",
            "[Account,Arn,UserId]",
            "--output",
            "text",
            "--cli-connect-timeout",
            "5",
            "--cli-read-timeout",
            "10",
        ])
        .output()
        .map_err(aws_error)
}

/// Formats a successful `caller_identity` output as the whoami report.
fn describe_identity(
    profile: &str,
    output: &Output,
    make_command: impl Fn() -> Command,
    settings: &Settings,
) -> io::Result<String> {
    let text = String::from_utf8_lossy(&output.stdout);
    let fields: Vec<_> = text.trim().split('\t').collect();
    if fields.len() != 3 || fields.iter().any(|v| v.is_empty() || *v == "None") {
        return Err(io::Error::other("unexpected AWS identity response"));
    }
    let region = match &settings.region {
        Some(region) => region.clone(),
        None => {
            let output = make_command()
                .args(["configure", "get", "region"])
                .output()
                .map_err(aws_error)?;
            if output.status.success() {
                String::from_utf8_lossy(&output.stdout).trim().to_owned()
            } else if output.status.code() == Some(1) && output.stderr.is_empty() {
                "(not configured)".into()
            } else {
                return Err(io::Error::other(format!(
                    "AWS region check failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
        }
    };
    Ok(format!(
        "Profile: {profile}\nAccount: {}\nARN: {}\nUser ID: {}\nRegion: {region}\n",
        fields[0], fields[1], fields[2]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_retries_invalid_selection_then_selects() {
        let mut input = &b"garbage\n9\n2\n"[..];
        let mut output = Vec::new();
        let profiles = vec!["default".into(), "dev".into()];
        assert_eq!(
            select_profile(&profiles, &[], &mut input, &mut output).unwrap(),
            "dev"
        );
    }

    #[test]
    fn zero_selects_custom_and_empty_names_retry() {
        let mut input = &b"0\n\n0\ncustom\n"[..];
        assert_eq!(
            select_profile(&["dev".into()], &[], &mut input, &mut Vec::new()).unwrap(),
            "custom"
        );
    }

    #[test]
    fn empty_list_allows_manual_input() {
        assert_eq!(
            select_profile(&[], &[], &mut &b"manual\n"[..], &mut Vec::new()).unwrap(),
            "manual"
        );
    }

    #[test]
    fn eof_and_cancel_abort() {
        for input in [&b""[..], &b"q\n"[..]] {
            assert!(
                select_profile(&["dev".into()], &[], &mut &input[..], &mut Vec::new()).is_err()
            );
        }
    }
}
