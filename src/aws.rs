mod history;

use crate::cli::AwsCommands;
use dialoguer::{Input, Select, theme::ColorfulTheme};
use std::{
    collections::HashSet,
    io::{self, BufRead, IsTerminal, Write},
    process::{Command, Stdio},
};

pub fn run(command: AwsCommands) -> io::Result<()> {
    match command {
        AwsCommands::Exec { profile, command } => {
            let profile = resolve_profile(profile)?;
            let mut child = Command::new(&command[0]);
            child.args(&command[1..]);
            configure_profile(&mut child, &profile);
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
            let profiles = profiles()?;
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
            let profile = profile.or_else(|| {
                if select {
                    None
                } else {
                    ["AWS_PROFILE", "AWS_DEFAULT_PROFILE"]
                        .into_iter()
                        .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
                }
            });
            let profile = resolve_profile(profile)?;
            if credentials_valid(&profile)? {
                eprintln!("AWS credentials are valid for profile {profile:?}; skipping login.");
            } else {
                eprintln!("Logging in with AWS profile {profile:?}...");
                let mut command = profile_command(&profile);
                command.args(["sso", "login", "--profile", &profile]);
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
                        "aws sso login failed for profile {profile:?}: {status}"
                    )));
                }
                eprintln!("SSO login completed for profile {profile:?}.");
            }
            history::remember(&profile);
            if export {
                writeln!(io::stdout().lock(), "{}", export_statement(&profile))?;
            } else if no_shell {
                eprintln!("To use this profile in your current Bash/Zsh shell:");
                eprintln!("  {}", export_statement(&profile));
            } else {
                return open_shell(&profile);
            }
            Ok(())
        }
    }
}

// Static credentials take precedence over AWS_PROFILE. Remove them only in the
// child processes (or explicitly requested export output), never in the parent process.
const CREDENTIAL_ENV: &[&str] = &[
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_SECURITY_TOKEN",
    "AWS_DEFAULT_PROFILE",
];

// Use the same credential environment for validation, login and the child shell.
fn profile_command(profile: &str) -> Command {
    let mut command = aws();
    configure_profile(&mut command, profile);
    command
}

fn credentials_valid(profile: &str) -> io::Result<bool> {
    let output = profile_command(profile)
        .args(["sts", "get-caller-identity", "--profile", profile])
        .stdout(Stdio::null())
        .output()
        .map_err(aws_error)?;
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

fn open_shell(profile: &str) -> io::Result<()> {
    let shell = std::env::var_os("SHELL")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    let mut command = Command::new(&shell);
    let _startup = crate::shell::configure(&mut command, &shell)?;
    command.arg("-i");
    configure_profile(&mut command, profile);
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

fn configure_profile(command: &mut Command, profile: &str) {
    command.env("AWS_PROFILE", profile);
    for name in CREDENTIAL_ENV {
        command.env_remove(name);
    }
}

fn resolve_profile(profile: Option<String>) -> io::Result<String> {
    let profile = match profile {
        Some(profile) => profile,
        None => {
            let mut profiles = profiles()?;
            history::sort(&mut profiles);
            if io::stdin().is_terminal() && io::stderr().is_terminal() {
                select_terminal_profile(&profiles)?
            } else {
                select_profile(&profiles, &mut io::stdin().lock(), &mut io::stderr().lock())?
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

fn select_terminal_profile(profiles: &[String]) -> io::Result<String> {
    let theme = ColorfulTheme::default();
    let mut visible = profiles.to_vec();
    while !profiles.is_empty() {
        let mut items = visible.clone();
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

fn aws() -> Command {
    let mut command = Command::new("aws");
    command.env("AWS_PAGER", "");
    command.env("AWS_CLI_AUTO_PROMPT", "off");
    command
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

fn profiles() -> io::Result<Vec<String>> {
    // AWS CLI handles config, credentials, default, sso-session sections and custom paths.
    let output = aws()
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

fn export_statement(profile: &str) -> String {
    format!(
        "unset {};\nexport AWS_PROFILE='{}'",
        CREDENTIAL_ENV.join(" "),
        profile.replace('\'', "'\\''")
    )
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

fn select_profile(
    profiles: &[String],
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
                writeln!(output, "  {}. {profile}", index + 1)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_retries_invalid_selection_then_selects() {
        let mut input = &b"garbage\n9\n2\n"[..];
        let mut output = Vec::new();
        let profiles = vec!["default".into(), "dev".into()];
        assert_eq!(
            select_profile(&profiles, &mut input, &mut output).unwrap(),
            "dev"
        );
    }

    #[test]
    fn zero_selects_custom_and_empty_names_retry() {
        let mut input = &b"0\n\n0\ncustom\n"[..];
        assert_eq!(
            select_profile(&["dev".into()], &mut input, &mut Vec::new()).unwrap(),
            "custom"
        );
    }

    #[test]
    fn empty_list_allows_manual_input() {
        assert_eq!(
            select_profile(&[], &mut &b"manual\n"[..], &mut Vec::new()).unwrap(),
            "manual"
        );
    }

    #[test]
    fn eof_and_cancel_abort() {
        for input in [&b""[..], &b"q\n"[..]] {
            assert!(select_profile(&["dev".into()], &mut &input[..], &mut Vec::new()).is_err());
        }
    }

    #[test]
    fn export_quotes_shell_metacharacters() {
        assert!(
            export_statement("a'b$(echo x)").ends_with("export AWS_PROFILE='a'\\''b$(echo x)'")
        );
        assert!(validate_profile("dev\nexport BAD=1").is_err());
        assert!(validate_profile("").is_err());
    }
}
