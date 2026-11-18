//! `tora aws sync`: generate `~/.aws/config` profiles for every account and
//! role an IAM Identity Center (SSO) session can access. This is the only
//! command that writes AWS configuration, and only inside its managed block.

use super::{Settings, status};
use crate::managed::{self, Change};
use dialoguer::{Select, theme::ColorfulTheme};
use std::{
    cell::Cell,
    collections::{BTreeSet, HashMap},
    env, fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::{Command, Output},
};

pub(super) struct Options {
    pub sso_session: Option<String>,
    pub region: Option<String>,
    pub dry_run: bool,
    pub yes: bool,
    pub no_browser: bool,
    pub use_device_code: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Session {
    name: String,
    region: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Account {
    id: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Profile {
    name: String,
    account_id: String,
    role: String,
}

pub(super) fn run(options: Options, settings: &Settings) -> io::Result<()> {
    let interactive = io::stdin().is_terminal() && io::stderr().is_terminal();
    // Fail before logging in when the change could never be confirmed.
    if !options.dry_run && !options.yes && !interactive {
        return Err(io::Error::other(
            "interactive terminal required to confirm changes; review them with --dry-run and pass --yes",
        ));
    }
    let path = config_path()?;
    let before = managed::read(&path)?;
    let text = before.clone().unwrap_or_default();
    let session = choose_session(sessions(&text), options.sso_session, &path, interactive)?;
    let sso_region = session.region.clone().ok_or_else(|| {
        io::Error::other(format!(
            "sso-session {:?} has no sso_region; fix it with `aws configure sso-session`",
            session.name
        ))
    })?;
    check_region(&sso_region)?;
    let region = options.region.unwrap_or_else(|| sso_region.clone());
    check_region(&region)?;

    let (begin, end) = markers(&session.name);
    let range = managed::block_range(&text, &begin, &end)?;
    let (old_block, outside) = match &range {
        Some(range) => (
            &text[range.clone()],
            format!("{}{}", &text[..range.start], &text[range.end..]),
        ),
        None => ("", text.clone()),
    };
    let mut existing = config_profiles(&outside);
    if let Some(credentials) = managed::read(&credentials_path()?)? {
        existing.extend(sections(&credentials).into_iter().map(|(name, _)| name));
    }

    let sso = Sso {
        settings,
        session: &session.name,
        region: &sso_region,
        no_browser: options.no_browser,
        use_device_code: options.use_device_code,
        logged_in: Cell::new(false),
        input_dir: input_dir()?,
    };
    let accounts = sso.accounts()?;
    let mut roles = Vec::new();
    for account in &accounts {
        for role in sso.roles(&account.id)? {
            if valid_role(&role) {
                roles.push((account.clone(), role));
            } else {
                eprintln!(
                    "Skipping a role of account {} with an unsupported name.",
                    account.id
                );
            }
        }
    }
    let (profiles, skipped) = generate(&roles, &existing);
    if !skipped.is_empty() {
        eprintln!(
            "Skipped (already defined outside tora's managed block): {}",
            skipped.join(", ")
        );
    }
    let block = render(&session.name, &region, &profiles);
    if options.dry_run {
        println!("{block}");
        return Ok(());
    }
    if profiles.is_empty() && range.is_none() {
        eprintln!(
            "No accounts or roles to add for sso-session {:?}; no files changed.",
            session.name
        );
        return Ok(());
    }
    let after = managed::replace_block(&text, range, &block);
    let change = Change::with_before(path, before, after)?;
    if !change.changed() {
        eprintln!(
            "AWS config is already up to date: {} ({} profiles)",
            change.path.display(),
            profiles.len()
        );
        return Ok(());
    }
    eprintln!("File to update: {}", change.path.display());
    eprint!("{}", managed::diff(old_block, &block, 2));
    eprintln!(
        "{} profiles for sso-session {:?}; only the managed block changes.",
        profiles.len(),
        session.name
    );
    if !managed::confirm("Update AWS config?", options.yes)? {
        eprintln!("Cancelled; no files changed.");
        return Ok(());
    }
    managed::apply(&[change])?;
    eprintln!("AWS config updated. List profiles with: tora aws profiles");
    Ok(())
}

/// `AWS_CONFIG_FILE`, else `~/.aws/config`, as AWS CLI resolves it.
fn config_path() -> io::Result<PathBuf> {
    aws_file("AWS_CONFIG_FILE", "config")
}

fn credentials_path() -> io::Result<PathBuf> {
    aws_file("AWS_SHARED_CREDENTIALS_FILE", "credentials")
}

fn aws_file(variable: &str, name: &str) -> io::Result<PathBuf> {
    let home = || {
        env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::other("HOME is required"))
    };
    match env::var_os(variable).filter(|value| !value.is_empty()) {
        Some(value) => {
            let path = PathBuf::from(value);
            match path.strip_prefix("~") {
                Ok(rest) => Ok(home()?.join(rest)),
                Err(_) => Ok(path),
            }
        }
        None => Ok(home()?.join(".aws").join(name)),
    }
}

/// INI sections as (header, key/value pairs), ignoring comments and nested values.
fn sections(text: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut sections: Vec<(String, HashMap<String, String>)> = Vec::new();
    for line in text.lines() {
        // Indented lines continue a nested value (e.g. `s3 =`), never a top-level key.
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        // Like Python's configparser (used by AWS CLI), a header runs to the
        // last `]`, so `[profile x] # note` is the section `profile x`.
        if let Some(header) = line
            .strip_prefix('[')
            .and_then(|l| Some(&l[..l.rfind(']')?]))
        {
            sections.push((header.trim().to_owned(), HashMap::new()));
        } else if let (Some(index), Some((_, values))) =
            (line.find(['=', ':']), sections.last_mut())
        {
            values.insert(
                line[..index].trim().to_ascii_lowercase(),
                line[index + 1..].trim().to_owned(),
            );
        }
    }
    sections
}

/// The name in a `<kind> NAME` header. AWS CLI splits these like a shell
/// (`[profile "dev"]` is `dev`), so surrounding quotes are removed.
fn named(header: &str, kind: &str) -> Option<String> {
    let name = header
        .strip_prefix(kind)
        .filter(|rest| rest.starts_with([' ', '\t']))?
        .trim();
    let unquoted = ['"', '\'']
        .iter()
        .find_map(|q| name.strip_prefix(*q).and_then(|n| n.strip_suffix(*q)))
        .unwrap_or(name);
    Some(unquoted.to_owned())
}

fn config_profiles(text: &str) -> BTreeSet<String> {
    sections(text)
        .into_iter()
        .filter_map(|(header, _)| {
            if header == "default" {
                Some(header)
            } else {
                named(&header, "profile")
            }
        })
        .collect()
}

fn sessions(text: &str) -> Vec<Session> {
    sections(text)
        .into_iter()
        .filter_map(|(header, values)| {
            let name = named(&header, "sso-session")?;
            Some(Session {
                name,
                region: values.get("sso_region").filter(|v| !v.is_empty()).cloned(),
            })
        })
        .collect()
}

fn choose_session(
    sessions: Vec<Session>,
    requested: Option<String>,
    path: &Path,
    interactive: bool,
) -> io::Result<Session> {
    let session = if let Some(requested) = requested {
        sessions
            .into_iter()
            .find(|session| session.name == requested)
            .ok_or_else(|| {
                io::Error::other(format!(
                    "sso-session {requested:?} not found in {}; create it with `aws configure sso-session`",
                    path.display()
                ))
            })?
    } else {
        match sessions.len() {
            0 => {
                return Err(io::Error::other(format!(
                    "no [sso-session] found in {}; create one with `aws configure sso-session`, then run tora aws sync again",
                    path.display()
                )));
            }
            1 => sessions.into_iter().next().expect("one session"),
            _ if interactive => {
                let names: Vec<_> = sessions.iter().map(|s| s.name.as_str()).collect();
                let index = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("SSO session (Esc/q to cancel)")
                    .items(&names)
                    .default(0)
                    .interact_opt()
                    .map_err(io::Error::other)?
                    .ok_or_else(|| io::Error::other("sso-session selection cancelled"))?;
                sessions.into_iter().nth(index).expect("selected session")
            }
            _ => {
                let names: Vec<_> = sessions.iter().map(|s| s.name.as_str()).collect();
                return Err(io::Error::other(format!(
                    "multiple sso-sessions found ({}); pass --sso-session NAME",
                    names.join(", ")
                )));
            }
        }
    };
    check_session_name(&session.name)?;
    eprintln!("Using sso-session {:?}.", session.name);
    Ok(session)
}

/// Session names end up in markers and `sso_session = …` lines.
fn check_session_name(name: &str) -> io::Result<()> {
    if name.is_empty()
        || name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "[]#;<>\"'\\".contains(c))
    {
        return Err(io::Error::other(format!(
            "unsupported sso-session name {name:?} for tora aws sync"
        )));
    }
    Ok(())
}

fn check_region(region: &str) -> io::Result<()> {
    if region.is_empty()
        || !region
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(io::Error::other(format!("invalid AWS region {region:?}")));
    }
    Ok(())
}

fn markers(session: &str) -> (String, String) {
    (
        format!("# >>> tora aws sync {session} >>>"),
        format!("# <<< tora aws sync {session} <<<"),
    )
}

/// IAM role names: alphanumerics and `+=,.@_-`.
fn valid_role(role: &str) -> bool {
    !role.is_empty()
        && role
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+=,.@_-".contains(c))
}

/// Profile-name part: keep ASCII alphanumerics, `_` and `.`; turn every other
/// run of characters (spaces, symbols, non-ASCII) into a single `-`.
fn normalize(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches(['-', '.']).to_owned()
}

/// Profiles named `<account>-<role>`, sorted by name. Accounts whose names
/// normalize to the same text get their account ID too, and names that are
/// already defined outside the managed block are skipped (returned second).
fn generate(
    roles: &[(Account, String)],
    existing: &BTreeSet<String>,
) -> (Vec<Profile>, Vec<String>) {
    let account_name = |account: &Account| {
        let name = normalize(&account.name);
        if name.is_empty() {
            account.id.clone()
        } else {
            name
        }
    };
    let mut ids_by_name: HashMap<String, BTreeSet<&str>> = HashMap::new();
    for (account, _) in roles {
        ids_by_name
            .entry(account_name(account))
            .or_default()
            .insert(&account.id);
    }
    let mut candidates: Vec<Profile> = roles
        .iter()
        .map(|(account, role)| {
            let name = account_name(account);
            let prefix = if ids_by_name[&name].len() > 1 && name != account.id {
                format!("{name}-{}", account.id)
            } else {
                name
            };
            Profile {
                name: format!("{prefix}-{role}"),
                account_id: account.id.clone(),
                role: role.clone(),
            }
        })
        .collect();
    candidates.sort_by(|a, b| a.name.cmp(&b.name).then(a.account_id.cmp(&b.account_id)));
    let mut profiles: Vec<Profile> = Vec::new();
    let mut skipped = Vec::new();
    for profile in candidates {
        if existing.contains(&profile.name) {
            skipped.push(profile.name);
        } else if profiles
            .last()
            .is_some_and(|last| last.name == profile.name)
        {
            eprintln!(
                "Skipping duplicate generated profile name {:?} (account {}).",
                profile.name, profile.account_id
            );
        } else {
            profiles.push(profile);
        }
    }
    (profiles, skipped)
}

fn render(session: &str, region: &str, profiles: &[Profile]) -> String {
    let (begin, end) = markers(session);
    let mut block = format!(
        "{begin}\n# Generated by tora aws sync --sso-session {session}. Edits inside this block are overwritten.\n# Define a profile with the same name outside this block to keep your own settings.\n"
    );
    for profile in profiles {
        block.push_str(&format!(
            "\n[profile {}]\nsso_session = {session}\nsso_account_id = {}\nsso_role_name = {}\nregion = {region}\n",
            profile.name, profile.account_id, profile.role
        ));
    }
    block.push_str(&end);
    block
}

struct Sso<'a> {
    settings: &'a Settings,
    session: &'a str,
    region: &'a str,
    no_browser: bool,
    use_device_code: bool,
    /// Log in at most once per run.
    logged_in: Cell<bool>,
    /// Private directory for the request files that carry the token.
    input_dir: PathBuf,
}

impl Sso<'_> {
    /// AWS CLI without profile selection: these calls use only the SSO token,
    /// so a broken or unrelated AWS_PROFILE must not affect them.
    fn command(&self) -> Command {
        let mut command = self.settings.command();
        command
            .env_remove("AWS_PROFILE")
            .env_remove("AWS_DEFAULT_PROFILE");
        command
    }

    fn login(&self) -> io::Result<()> {
        self.logged_in.set(true);
        super::sso_login(
            self.command(),
            &["--sso-session", self.session],
            &format!("sso-session {:?}", self.session),
            self.no_browser,
            self.use_device_code,
        )
    }

    fn accounts(&self) -> io::Result<Vec<Account>> {
        let output = self.query(
            "list-accounts",
            None,
            "accountList[].[accountId,accountName]",
        )?;
        let mut accounts = Vec::new();
        for line in output.lines().filter(|line| !line.is_empty()) {
            let (id, name) = line.split_once('\t').unwrap_or((line, ""));
            if id.len() != 12 || !id.bytes().all(|b| b.is_ascii_digit()) {
                return Err(io::Error::other(
                    "unexpected aws sso list-accounts response",
                ));
            }
            let name = if name == "None" { "" } else { name };
            accounts.push(Account {
                id: id.to_owned(),
                name: name.to_owned(),
            });
        }
        Ok(accounts)
    }

    fn roles(&self, account_id: &str) -> io::Result<Vec<String>> {
        let output = self.query(
            "list-account-roles",
            Some(account_id),
            "roleList[].[roleName]",
        )?;
        Ok(output
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// Runs an `aws sso` query, logging in (once per run) when the cached token
    /// is missing, about to expire, or rejected by AWS.
    fn query(&self, operation: &str, account_id: Option<&str>, query: &str) -> io::Result<String> {
        loop {
            let Some(token) = access_token(self.session) else {
                if self.logged_in.get() {
                    return Err(io::Error::other(format!(
                        "no valid SSO token cached for sso-session {:?} after login",
                        self.session
                    )));
                }
                self.login()?;
                continue;
            };
            let error = match self.call(operation, account_id, query, &token)? {
                Ok(output) => return Ok(output),
                Err(error) => error,
            };
            let rejected = error
                .to_ascii_lowercase()
                .contains("(unauthorizedexception)");
            if !rejected || self.logged_in.get() {
                return Err(io::Error::other(format!(
                    "aws sso {operation} failed for sso-session {:?}: {}",
                    self.session,
                    error.trim()
                )));
            }
            self.login()?;
        }
    }

    /// Ok(Err(stderr)) when AWS CLI reports a failure.
    fn call(
        &self,
        operation: &str,
        account_id: Option<&str>,
        query: &str,
        token: &str,
    ) -> io::Result<Result<String, String>> {
        // Pass the token in a private file, never on the command line where
        // other local users could read it from the process list. AWS CLI
        // cannot read --cli-input-json from a pipe.
        let mut input = tempfile::Builder::new()
            .prefix(INPUT_PREFIX)
            .tempfile_in(&self.input_dir)?;
        let account = account_id
            .map(|id| format!(", \"accountId\": \"{id}\""))
            .unwrap_or_default();
        writeln!(input, "{{\"accessToken\": \"{token}\"{account}}}")?;
        input.as_file().sync_all()?;
        let output: Output = self
            .command()
            .args([
                "sso",
                operation,
                "--region",
                self.region,
                "--cli-input-json",
            ])
            .arg(format!("file://{}", input.path().display()))
            .args(["--query", query, "--output", "text"])
            .output()
            .map_err(super::aws_error)?;
        if !output.status.success() {
            return Ok(Err(String::from_utf8_lossy(&output.stderr).into_owned()));
        }
        String::from_utf8(output.stdout)
            .map(Ok)
            .map_err(|_| io::Error::other(format!("aws sso {operation} returned invalid UTF-8")))
    }
}

const INPUT_PREFIX: &str = "sso-request-";

/// `$TORA_HOME/tmp`, private to the user. Request files left behind by an
/// interrupted run (Ctrl-C while AWS CLI runs) are removed here; recent ones
/// may belong to a concurrent run and are kept.
fn input_dir() -> io::Result<PathBuf> {
    let dir = crate::paths::home()?.join("tmp");
    fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    for entry in fs::read_dir(&dir)?.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|time| time.elapsed().is_ok_and(|age| age.as_secs() > 600));
        if stale
            && entry
                .file_name()
                .to_string_lossy()
                .starts_with(INPUT_PREFIX)
        {
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(dir)
}

/// The cached access token for an sso-session while it stays valid for ten
/// more minutes, enough to list many accounts one by one. The token is a
/// secret: never print or log it.
fn access_token(session: &str) -> Option<String> {
    let content = status::read_cache(session)?;
    let expires = status::parse_timestamp(status::json_string(&content, "expiresAt")?)?;
    if expires <= status::now() + 600 {
        return None;
    }
    let token = status::json_string(&content, "accessToken")?;
    // json_string rejects escapes, so the token is safe to embed in JSON.
    (!token.is_empty() && !token.chars().any(char::is_control)).then(|| token.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, name: &str) -> Account {
        Account {
            id: id.into(),
            name: name.into(),
        }
    }

    #[test]
    fn normalizes_spaces_symbols_and_non_ascii() {
        assert_eq!(normalize("Prod"), "Prod");
        assert_eq!(normalize("  Dev Team (JP)! "), "Dev-Team-JP");
        assert_eq!(normalize("a__b.c"), "a__b.c");
        assert_eq!(normalize("本番 環境"), "");
        assert_eq!(normalize("..x.."), "x");
    }

    #[test]
    fn generates_sorted_unique_names_and_skips_existing() {
        let roles = vec![
            (account("111111111111", "Prod"), "ReadOnly".to_owned()),
            (account("111111111111", "Prod"), "Admin".to_owned()),
            (account("222222222222", "Dev Team"), "Admin".to_owned()),
            (account("333333333333", "Dev-Team!"), "Admin".to_owned()),
            (account("444444444444", "本番"), "Admin".to_owned()),
        ];
        let existing = BTreeSet::from(["Prod-Admin".to_owned()]);
        let (profiles, skipped) = generate(&roles, &existing);
        let names: Vec<_> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "444444444444-Admin",
                "Dev-Team-222222222222-Admin",
                "Dev-Team-333333333333-Admin",
                "Prod-ReadOnly",
            ]
        );
        assert_eq!(skipped, ["Prod-Admin"]);
        assert_eq!(profiles[3].account_id, "111111111111");
        assert_eq!(profiles[3].role, "ReadOnly");
    }

    #[test]
    fn reads_profiles_and_sessions_from_ini() {
        let text = "[default]\nregion = x\n[profile  dev ]\n  s3 =\n    sso_region = no\n[sso-session corp]\nsso_region = us-east-1\nsso_start_url = https://x\n# [profile commented]\n[sso-session other]\n[services s]\n[profiler x]\n";
        assert_eq!(
            config_profiles(text),
            BTreeSet::from(["default".to_owned(), "dev".to_owned()])
        );
        assert_eq!(
            sessions(text),
            [
                Session {
                    name: "corp".into(),
                    region: Some("us-east-1".into())
                },
                Session {
                    name: "other".into(),
                    region: None
                },
            ]
        );
    }

    #[test]
    fn parses_headers_and_keys_like_aws_cli() {
        let text = "[profile a] # note\n[profile \"b\"]\n[profile 'c']\n[sso-session \"corp\"]\nSSO_Region: us-west-2\n";
        assert_eq!(
            config_profiles(text),
            BTreeSet::from(["a".to_owned(), "b".to_owned(), "c".to_owned()])
        );
        assert_eq!(
            sessions(text),
            [Session {
                name: "corp".into(),
                region: Some("us-west-2".into())
            }]
        );
    }

    #[test]
    fn renders_a_managed_block() {
        let profiles = [Profile {
            name: "Prod-Admin".into(),
            account_id: "111111111111".into(),
            role: "Admin".into(),
        }];
        let block = render("corp", "ap-northeast-1", &profiles);
        let (begin, end) = markers("corp");
        assert!(block.starts_with(&begin));
        assert!(block.ends_with(&end));
        assert!(block.contains(
            "\n[profile Prod-Admin]\nsso_session = corp\nsso_account_id = 111111111111\nsso_role_name = Admin\nregion = ap-northeast-1\n"
        ));
        assert!(
            managed::block_range(&block, &begin, &end)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn rejects_unsafe_names_and_regions() {
        for name in ["", "a b", "a]", "x#", "a>b"] {
            assert!(check_session_name(name).is_err(), "{name:?}");
        }
        assert!(check_session_name("corp-sso.1").is_ok());
        assert!(check_region("ap-northeast-1").is_ok());
        assert!(check_region("us east").is_err());
        assert!(!valid_role("Admin\nx"));
        assert!(valid_role("AWSReservedSSO_Admin+x=1,a.b@c-d"));
    }
}
