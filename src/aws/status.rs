use super::Settings;
use std::{
    fs,
    io::{self, Read, Write},
    path::PathBuf,
    process::Stdio,
    time::{SystemTime, UNIX_EPOCH},
};

// Token cache files are small; never read unbounded data from the cache directory.
const MAX_BYTES: u64 = 64 * 1024;

/// `whoami` for the selected profile plus the SSO token expiry. Never logs in.
pub(super) fn run(profile: &str, settings: &Settings) -> io::Result<()> {
    let make_command = || super::explicit_command(profile, settings);
    let output = super::caller_identity(make_command())?;
    let report = if super::authenticated(profile, &output)? {
        Some(super::describe_identity(
            profile,
            &output,
            make_command,
            settings,
        )?)
    } else {
        None
    };
    // Read the cache after the check: AWS CLI may have refreshed the token.
    let expiry = describe_expiry(sso_expiry(profile, settings).as_ref(), now());

    let mut out = io::stdout().lock();
    match &report {
        Some(report) => write!(out, "{report}")?,
        None => writeln!(out, "Profile: {profile}")?,
    }
    writeln!(out, "SSO expires: {expiry}")?;
    if report.is_some() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "not logged in or the session has expired for profile {profile:?}; {}",
        login_hint(profile)
    )))
}

/// The `SSO expires:` text for doctor (`unknown` when it cannot be determined);
/// an error when the token has already expired.
pub(super) fn expiry(profile: &str, settings: &Settings) -> io::Result<String> {
    let expiry = sso_expiry(profile, settings);
    let now = now();
    let text = describe_expiry(expiry.as_ref(), now);
    if expiry.is_some_and(|expiry| expiry.epoch <= now) {
        return Err(io::Error::other(format!(
            "SSO token for profile {profile:?} expires: {text}; {}",
            login_hint(profile)
        )));
    }
    Ok(text)
}

fn login_hint(profile: &str) -> String {
    // Only echo names that are safe to paste into a shell.
    let safe = |c: char| c.is_ascii_alphanumeric() || "_.-@".contains(c);
    let argument = if profile.chars().all(safe) && !profile.starts_with('-') {
        format!(" {profile}")
    } else {
        String::new()
    };
    format!("run `tora aws sso{argument}` to log in")
}

pub(super) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

struct Expiry {
    /// Validated timestamp text from the cache file (digits and separators only).
    text: String,
    epoch: i64,
    refreshable: bool,
}

fn describe_expiry(expiry: Option<&Expiry>, now: i64) -> String {
    let Some(expiry) = expiry else {
        return "unknown".into();
    };
    let remaining = expiry.epoch - now;
    if remaining <= 0 {
        return format!("{} (expired)", expiry.text);
    }
    let note = if expiry.refreshable {
        ", refreshed automatically while the SSO session lasts"
    } else {
        ""
    };
    format!("{} ({} remaining{note})", expiry.text, duration(remaining))
}

fn duration(seconds: i64) -> String {
    let minutes = seconds / 60;
    match (minutes / 60, minutes % 60) {
        (0, 0) => "less than 1m".into(),
        (0, minutes) => format!("{minutes}m"),
        (hours, minutes) => format!("{hours}h {minutes}m"),
    }
}

/// Any failure means "unknown": the status command must not fail because the
/// cache is missing, unreadable or in an unexpected format.
fn sso_expiry(profile: &str, settings: &Settings) -> Option<Expiry> {
    // AWS CLI names the token cache after the sso-session name, or after the
    // start URL for legacy profiles without an sso-session.
    let key = configured(profile, settings, "sso_session")
        .or_else(|| configured(profile, settings, "sso_start_url"))?;
    let content = read_cache(&key)?;
    // The file also holds the access token; extract the timestamp only and
    // never print or log anything else from it.
    let text = json_string(&content, "expiresAt")?;
    Some(Expiry {
        epoch: parse_timestamp(text)?,
        text: text.to_owned(),
        refreshable: json_string(&content, "refreshToken").is_some(),
    })
}

/// The AWS CLI SSO token cache file for an sso-session name (or legacy start URL).
/// It holds the access token: callers must never print or log it.
pub(super) fn read_cache(key: &str) -> Option<String> {
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty())?;
    let path = PathBuf::from(home)
        .join(".aws/sso/cache")
        .join(format!("{}.json", sha1_hex(key.as_bytes())));
    let mut content = String::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_BYTES + 1)
        .read_to_string(&mut content)
        .ok()?;
    (content.len() as u64 <= MAX_BYTES).then_some(content)
}

fn configured(profile: &str, settings: &Settings, name: &str) -> Option<String> {
    let output = super::explicit_command(profile, settings)
        .args(["configure", "get", name])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// Returns the raw value of a top-level-style `"key": "value"` string member.
/// Values with escapes are rejected; timestamps and tokens never need them here.
pub(super) fn json_string<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let mut rest = json;
    while let Some(index) = rest.find(&needle) {
        rest = &rest[index + needle.len()..];
        if let Some(value) = rest.trim_start().strip_prefix(':') {
            let value = value.trim_start().strip_prefix('"')?;
            let value = &value[..value.find('"')?];
            return (!value.contains('\\')).then_some(value);
        }
    }
    None
}

/// Parses `YYYY-MM-DDTHH:MM:SS[.fff]` followed by `Z`, `UTC` or `±HH:MM`.
pub(super) fn parse_timestamp(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 || !text.is_ascii() {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let digits = &text[range];
        digits
            .bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| digits.parse().ok())?
    };
    let separators = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    if separators.iter().any(|&(index, byte)| bytes[index] != byte) {
        return None;
    }
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut zone = &text[19..];
    if let Some(fraction) = zone.strip_prefix('.') {
        zone = fraction.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match zone {
        "Z" | "UTC" => 0,
        _ => {
            let sign = match zone.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            if zone.len() != 6 || zone.as_bytes()[3] != b':' {
                return None;
            }
            let hours: i64 = zone[1..3].parse().ok()?;
            let minutes: i64 = zone[4..6].parse().ok()?;
            sign * (hours * 3600 + minutes * 60)
        }
    };
    // Days from the civil date (proleptic Gregorian calendar, 1970-01-01 = 0).
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// SHA-1 is only used to derive AWS CLI's cache file name, not for security.
fn sha1_hex(data: &[u8]) -> String {
    let mut state: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&(data.len() as u64 * 8).to_be_bytes());
    for block in message.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (word, bytes) in w.iter_mut().zip(block.chunks_exact(4)) {
            *word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            (e, d, c, b, a) = (d, c, b.rotate_left(30), a, next);
        }
        for (value, add) in state.iter_mut().zip([a, b, c, d, e]) {
            *value = value.wrapping_add(add);
        }
    }
    state.iter().map(|word| format!("{word:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_matches_known_digests() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha1_hex(b"https://example.awsapps.com/start"),
            "e8be5486177c5b5392bd9aa76563515b29358e6e"
        );
        // Padding spills into a second block.
        assert_eq!(
            sha1_hex(&[b'a'; 60]),
            "13d956033d9af449bfe2c4ef78c17c20469c4bf1"
        );
    }

    #[test]
    fn timestamps_accept_aws_cli_formats_only() {
        assert_eq!(parse_timestamp("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_timestamp("2024-02-29T12:34:56Z"), Some(1_709_210_096));
        assert_eq!(
            parse_timestamp("2024-02-29T12:34:56UTC"),
            Some(1_709_210_096)
        );
        assert_eq!(
            parse_timestamp("2024-02-29T12:34:56.123Z"),
            Some(1_709_210_096)
        );
        assert_eq!(
            parse_timestamp("2024-02-29T21:34:56+09:00"),
            Some(1_709_210_096)
        );
        for invalid in [
            "",
            "2024-02-29",
            "2024-02-29T12:34:56",
            "2024-13-01T00:00:00Z",
            "2024-02-29 12:34:56Z",
            "+024-02-29T12:34:56Z",
            "2024-02-29T12:34:56Zjunk",
        ] {
            assert_eq!(parse_timestamp(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn json_string_reads_only_the_requested_member() {
        let json = r#"{"note": "expiresAt", "accessToken": "secret", "expiresAt" : "2099-01-01T00:00:00Z"}"#;
        assert_eq!(json_string(json, "expiresAt"), Some("2099-01-01T00:00:00Z"));
        assert_eq!(json_string(json, "refreshToken"), None);
        assert_eq!(json_string(r#"{"expiresAt": 1}"#, "expiresAt"), None);
        assert_eq!(json_string(r#"{"expiresAt": "a\"b"}"#, "expiresAt"), None);
    }

    #[test]
    fn expiry_shows_remaining_time_or_expired() {
        let expiry = |epoch, refreshable| Expiry {
            text: "T".into(),
            epoch,
            refreshable,
        };
        assert_eq!(describe_expiry(None, 0), "unknown");
        assert_eq!(
            describe_expiry(Some(&expiry(11_580, false)), 0),
            "T (3h 13m remaining)"
        );
        assert_eq!(
            describe_expiry(Some(&expiry(130, false)), 100),
            "T (less than 1m remaining)"
        );
        assert!(describe_expiry(Some(&expiry(700, true)), 100).starts_with("T (10m remaining, "));
        assert_eq!(
            describe_expiry(Some(&expiry(100, true)), 100),
            "T (expired)"
        );
    }
}
