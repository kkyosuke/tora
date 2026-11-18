//! `tora aws prompt`: a short profile indicator for shell prompts.
//!
//! It runs on every prompt, so it must stay fast and quiet: it never starts
//! AWS CLI, uses the network, asks questions or writes to stderr. Anything it
//! cannot determine is simply left out of the output.

use super::{settings, status};
use crate::config;
use std::{
    env,
    io::{self, Write},
    path::PathBuf,
};

// AWS config files are small; bound the read so a prompt can never stall.
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// Prints the indicator, or nothing when no profile is selected. Never fails.
pub(super) fn run(format: Option<&str>) {
    if let Some(text) = render(format) {
        // A closed pipe only means the shell stopped listening.
        let _ = writeln!(io::stdout().lock(), "{text}");
    }
}

fn render(format: Option<&str>) -> Option<String> {
    // The same order as other tora commands: environment, then aws.profile.
    // A broken tora config only hides the saved default; it never fails the prompt.
    let profile = settings::env_value(&["AWS_PROFILE", "AWS_DEFAULT_PROFILE"])
        .or_else(|| config::load().ok()?.profile)?;
    if profile.chars().any(char::is_control) {
        return None;
    }
    let remaining = remaining(&profile, status::now());
    Some(match format {
        Some(format) => expand(format, &profile, remaining.as_deref().unwrap_or("")),
        None => match remaining {
            Some(remaining) => format!("{profile} ({remaining})"),
            None => profile,
        },
    })
}

/// The access token's time left (`52m`, `3h 13m`, `<1m`) or `expired`, like
/// `tora aws status` and doctor. None when unknown.
fn remaining(profile: &str, now: i64) -> Option<String> {
    let config = status::read_small(&config_path()?, MAX_CONFIG_BYTES)?;
    // The same cache keys as `tora aws status`, read without AWS CLI.
    let key = profile_value(&config, profile, "sso_session")
        .or_else(|| profile_value(&config, profile, "sso_start_url"))?;
    let expiry = status::cached_expiry(key)?;
    let seconds = expiry.epoch - now;
    Some(match seconds {
        ..=0 => "expired".into(),
        // `less than 1m` is too long for a prompt.
        1..60 => "<1m".into(),
        _ => status::duration(seconds),
    })
}

/// AWS_CONFIG_FILE (with `~` expanded, like AWS CLI) or `~/.aws/config`.
fn config_path() -> Option<PathBuf> {
    let Some(path) = env::var_os("AWS_CONFIG_FILE").filter(|path| !path.is_empty()) else {
        return Some(status::home()?.join(".aws/config"));
    };
    let path = PathBuf::from(path);
    match path.strip_prefix("~") {
        Ok(rest) => Some(status::home()?.join(rest)),
        Err(_) => Some(path),
    }
}

/// Looks up `key` for a profile as AWS CLI reads its config file: sections
/// `[default]` and `[profile NAME]` (the later one wins when both exist),
/// `key = value` or `key: value` with case-insensitive keys, comment lines
/// starting with `#` or `;`, and lines indented deeper than the previous key
/// continuing its value (nested settings such as `s3 =`).
fn profile_value<'a>(config: &'a str, profile: &str, key: &str) -> Option<&'a str> {
    let mut selected = false;
    let mut found = None;
    // Indentation of the section's last key; deeper lines continue its value.
    let mut key_indent = None;
    for line in config.lines() {
        let indent = line.len() - line.trim_start().len();
        let line = line.trim();
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if key_indent.is_some_and(|key_indent| indent > key_indent) {
            continue;
        }
        if let Some(header) = line.strip_prefix('[') {
            selected = header
                .rfind(']')
                .and_then(|end| section_profile(&header[..end]))
                == Some(profile);
            key_indent = None;
            if selected {
                // AWS CLI replaces, not merges, a profile defined twice.
                found = None;
            }
            continue;
        }
        let Some((name, value)) = line.split_once(['=', ':']) else {
            continue;
        };
        key_indent = Some(indent);
        let value = value.trim();
        if selected && found.is_none() && name.trim().eq_ignore_ascii_case(key) {
            found = (!value.is_empty()).then_some(value);
        }
    }
    found
}

/// The profile a config section header names: `default`, `profile NAME` or
/// `profile "NAME WITH SPACES"`. Other sections (sso-session, services) are not profiles.
fn section_profile(header: &str) -> Option<&str> {
    if header == "default" {
        return Some(header);
    }
    let name = header.strip_prefix("profile")?;
    if !name.starts_with([' ', '\t']) {
        return None;
    }
    let name = name.trim();
    let quoted = ['"', '\''].into_iter().find_map(|quote| {
        name.strip_prefix(quote)
            .and_then(|inner| inner.strip_suffix(quote))
    });
    match quoted {
        Some(inner) => (!inner.is_empty()).then_some(inner),
        None => (!name.is_empty() && !name.contains(char::is_whitespace)).then_some(name),
    }
}

/// Replaces `{profile}` and `{remaining}` in one pass, so a profile name that
/// itself contains a placeholder is printed literally.
fn expand(format: &str, profile: &str, remaining: &str) -> String {
    let placeholders = [("{profile}", profile), ("{remaining}", remaining)];
    let mut text = String::new();
    let mut rest = format;
    while let Some(start) = rest.find('{') {
        text.push_str(&rest[..start]);
        rest = &rest[start..];
        match placeholders
            .iter()
            .find(|(placeholder, _)| rest.starts_with(placeholder))
        {
            Some((placeholder, value)) => {
                text.push_str(value);
                rest = &rest[placeholder.len()..];
            }
            None => {
                text.push('{');
                rest = &rest[1..];
            }
        }
    }
    text.push_str(rest);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "\
# comment
[default]
sso_start_url = https://default.awsapps.com/start

[profile dev]
region = ap-northeast-1
s3 =
  sso_session = nested
SSO_Session = corp
sso_start_url = https://dev.awsapps.com/start

[sso-session corp]
sso_start_url = https://corp.awsapps.com/start

[profile legacy]
; comment
sso_start_url: https://legacy.awsapps.com/start
sso_session =

[profile \"with space\"]
sso_session=spaced

[profiledev]
sso_session = wrong

[ profile spaced ]
sso_session = rejected
";

    #[test]
    fn profile_values_follow_aws_config_sections() {
        let value = |profile, key| profile_value(CONFIG, profile, key);
        assert_eq!(value("dev", "sso_session"), Some("corp"));
        assert_eq!(value("dev", "region"), Some("ap-northeast-1"));
        assert_eq!(
            value("default", "sso_start_url"),
            Some("https://default.awsapps.com/start")
        );
        assert_eq!(
            value("legacy", "sso_start_url"),
            Some("https://legacy.awsapps.com/start")
        );
        assert_eq!(value("legacy", "sso_session"), None);
        assert_eq!(value("with space", "sso_session"), Some("spaced"));
        // Neither the sso-session section nor `[profiledev]` is a profile.
        assert_eq!(value("corp", "sso_start_url"), None);
        assert_eq!(value("profiledev", "sso_session"), None);
        assert_eq!(value("missing", "sso_session"), None);
        // AWS CLI rejects whitespace inside the brackets but accepts indented sections.
        assert_eq!(value("spaced", "sso_session"), None);
        let indented = "  [profile indented]\n  region = x\n  sso_session = i\n    nested = n\n";
        assert_eq!(
            profile_value(indented, "indented", "sso_session"),
            Some("i")
        );
        assert_eq!(profile_value(indented, "indented", "nested"), None);
        // An indented line after a key continues its value, even a header.
        let continued = "[profile a]\nregion = x\n\n  [profile b]\nsso_session = s\n";
        assert_eq!(profile_value(continued, "a", "sso_session"), Some("s"));
        assert_eq!(profile_value(continued, "b", "sso_session"), None);
    }

    #[test]
    fn later_default_section_replaces_the_earlier_one() {
        let config = "[default]\nsso_session = d\n[profile default]\nregion = x\n";
        assert_eq!(profile_value(config, "default", "sso_session"), None);
        let config = "[profile default]\nregion = x\n[default]\nsso_session = d\n";
        assert_eq!(profile_value(config, "default", "sso_session"), Some("d"));
    }

    #[test]
    fn section_headers_name_profiles_like_aws_cli() {
        assert_eq!(section_profile("default"), Some("default"));
        assert_eq!(section_profile("profile dev"), Some("dev"));
        assert_eq!(section_profile("profile   dev"), Some("dev"));
        assert_eq!(section_profile("profile 'a b'"), Some("a b"));
        for header in [
            "profile",
            "profile ",
            "profiledev",
            "profile a b",
            "sso-session x",
        ] {
            assert_eq!(section_profile(header), None, "{header:?}");
        }
    }

    #[test]
    fn format_expands_known_placeholders_once() {
        assert_eq!(
            expand("[{profile}|{remaining}] {other} {", "dev", "52m"),
            "[dev|52m] {other} {"
        );
        assert_eq!(expand("{profile}", "{remaining}", "52m"), "{remaining}");
        assert_eq!(expand("aws:{profile}{remaining}", "dev", ""), "aws:dev");
    }
}
