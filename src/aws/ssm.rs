use super::{Settings, history};
use dialoguer::{Input, Select, theme::ColorfulTheme};
use std::{
    collections::HashMap,
    io::{self, BufRead, IsTerminal, Write},
    path::Path,
    process::Stdio,
};
use unicode_width::UnicodeWidthStr;

const PLUGIN: &str = "session-manager-plugin";
// DescribeInstances accepts at most 200 values per filter.
const FILTER_CHUNK: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Instance {
    id: String,
    name: String,
    status: String,
    platform: String,
}

impl Instance {
    fn display_name(&self) -> &str {
        if self.name.is_empty() {
            "-"
        } else {
            &self.name
        }
    }

    /// Search covers what the user sees as data, not padding or history markers.
    fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        [&self.name, &self.id, &self.platform]
            .iter()
            .any(|field| field.to_lowercase().contains(&query))
    }
}

pub(super) fn run(
    profile: Option<String>,
    target: Option<String>,
    settings: &Settings,
    no_browser: bool,
    use_device_code: bool,
) -> io::Result<()> {
    if let Some(target) = &target {
        check_target(target)?;
    }
    if let Some(profile) = profile.as_deref().filter(|p| looks_like_instance_id(p)) {
        return Err(io::Error::other(format!(
            "{profile:?} looks like an instance ID, not a profile; use --target {profile}"
        )));
    }
    // Fail before any login or selection when the session could never start.
    if !plugin_installed() {
        return Err(io::Error::other(format!(
            "{PLUGIN} not found in PATH; install the Session Manager plugin for AWS CLI first \
             (https://docs.aws.amazon.com/systems-manager/latest/userguide/session-manager-working-with-install-plugin.html)"
        )));
    }
    let profile = super::resolve_profile(profile, settings)?;
    super::ensure_login(&profile, settings, no_browser, use_device_code)?;
    history::remember(&profile);

    let target = match target {
        Some(target) => target,
        None => {
            let mut instances = list_instances(&profile, settings)?;
            if instances.is_empty() {
                return Err(io::Error::other(format!(
                    "no Session Manager managed instances found for profile {profile:?}; \
                     check the profile's region, or pass --target"
                )));
            }
            let recent = recent_targets(&profile);
            history::sort_by_recent(&mut instances, &recent, |i| &i.id);
            let labels = labels(&instances, &recent);
            let index = if io::stdin().is_terminal() && io::stderr().is_terminal() {
                select_terminal(&instances, &labels)?
            } else {
                select_piped(
                    &instances,
                    &labels,
                    &mut io::stdin().lock(),
                    &mut io::stderr().lock(),
                )?
            };
            instances.swap_remove(index).id
        }
    };
    history::remember_instance(&profile, &target);

    eprintln!("Starting Session Manager session on {target} with profile {profile:?}...");
    let mut command = settings.profile_command(&profile);
    command.args(["ssm", "start-session", "--target", &target]);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Replace tora so the session owns the terminal, signals and exit status.
        Err(super::aws_error(command.exec()))
    }
    #[cfg(not(unix))]
    {
        let status = command.status().map_err(super::aws_error)?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

/// Session Manager targets such as `i-…`, `mi-…` or `ecs:…`. The first character must not
/// be `-` so a target can never be parsed as an AWS CLI option.
pub(super) fn valid_target(target: &str) -> bool {
    target.len() <= 256
        && target.starts_with(|c: char| c.is_ascii_alphanumeric())
        && target
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}

fn check_target(target: &str) -> io::Result<()> {
    if valid_target(target) {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "invalid Session Manager target {target:?}; expected an ID such as i-0123456789abcdef0"
        )))
    }
}

/// EC2 (`i-`) and managed node (`mi-`) IDs: a prefix and 8 or 17 hex digits.
fn looks_like_instance_id(value: &str) -> bool {
    ["i-", "mi-"].iter().any(|prefix| {
        value.strip_prefix(prefix).is_some_and(|hex| {
            matches!(hex.len(), 8 | 17) && hex.chars().all(|c| c.is_ascii_hexdigit())
        })
    })
}

fn plugin_installed() -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(PLUGIN)))
    })
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file() || path.with_extension("exe").is_file()
    }
}

/// Targets used with this profile, most recent first.
fn recent_targets(profile: &str) -> Vec<String> {
    history::INSTANCES
        .recent()
        .into_iter()
        .filter_map(|entry| {
            let (p, target) = entry.split_once('\t')?;
            (p == profile).then(|| target.to_owned())
        })
        .collect()
}

fn text_query(profile: &str, settings: &Settings, args: &[&str]) -> io::Result<String> {
    let output = settings
        .profile_command(profile)
        .args(args)
        .args(["--output", "text"])
        .stderr(Stdio::piped())
        .output()
        .map_err(super::aws_error)?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "aws {} failed for profile {profile:?}: {}\n{}",
            args[..2].join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn list_instances(profile: &str, settings: &Settings) -> io::Result<Vec<Instance>> {
    let output = text_query(
        profile,
        settings,
        &[
            "ssm",
            "describe-instance-information",
            "--query",
            "InstanceInformationList[].[InstanceId,PingStatus,PlatformName,ComputerName]",
        ],
    )?;
    let mut instances = parse_instances(&output);
    let ec2_ids: Vec<&str> = instances
        .iter()
        .map(|i| i.id.as_str())
        .filter(|id| id.starts_with("i-"))
        .collect();
    let mut names = HashMap::new();
    for chunk in ec2_ids.chunks(FILTER_CHUNK) {
        let filter = format!("Name=instance-id,Values={}", chunk.join(","));
        match text_query(
            profile,
            settings,
            &[
                "ec2",
                "describe-instances",
                "--filters",
                &filter,
                "--query",
                "Reservations[].Instances[].[InstanceId,Tags[?Key=='Name'] | [0].Value]",
            ],
        ) {
            Ok(output) => names.extend(parse_names(&output)),
            Err(error) => {
                // Names are a convenience; Session Manager only needs the ID.
                eprintln!("tora: could not read EC2 Name tags; showing host names\n{error}");
                break;
            }
        }
    }
    for instance in &mut instances {
        if let Some(name) = names.remove(&instance.id) {
            instance.name = name;
        }
    }
    // Online instances first, then by name, so the list is stable between runs.
    instances.sort_by(|a, b| {
        (a.status != "Online", &a.name, &a.id).cmp(&(b.status != "Online", &b.name, &b.id))
    });
    Ok(instances)
}

/// Characters that can reorder or hide text in a terminal: controls, bidirectional
/// overrides/isolates, zero-width characters and other invisible format characters.
fn is_unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2069}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
        )
}

/// AWS CLI text output prints null as `None`; unsafe characters from tags are replaced
/// so names cannot rewrite or reorder the menu.
fn field(value: Option<&str>) -> String {
    match value.map(str::trim) {
        None | Some("") | Some("None") => String::new(),
        Some(value) => value
            .chars()
            .map(|c| if is_unsafe_char(c) { '?' } else { c })
            .collect(),
    }
}

fn parse_instances(output: &str) -> Vec<Instance> {
    let mut instances: Vec<Instance> = Vec::new();
    for line in output.lines() {
        let mut fields = line.split('\t');
        let id = field(fields.next());
        if !valid_target(&id) || instances.iter().any(|i| i.id == id) {
            continue;
        }
        let status = field(fields.next());
        let platform = field(fields.next());
        let computer = field(fields.next());
        instances.push(Instance {
            id,
            name: computer,
            status,
            platform,
        });
    }
    instances
}

fn parse_names(output: &str) -> HashMap<String, String> {
    output
        .lines()
        .filter_map(|line| {
            let (id, name) = line.split_once('\t')?;
            let name = field(Some(name));
            (!name.is_empty()).then(|| (id.trim().to_owned(), name))
        })
        .collect()
}

/// Pads by terminal display width so full-width (e.g. Japanese) names stay aligned.
fn pad(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}

fn labels(instances: &[Instance], recent: &[String]) -> Vec<String> {
    let width = |f: fn(&Instance) -> &str| instances.iter().map(|i| f(i).width()).max();
    let name_w = width(Instance::display_name).unwrap_or(1);
    let id_w = width(|i| &i.id).unwrap_or(0);
    let status_w = width(|i| &i.status).unwrap_or(0);
    instances
        .iter()
        .map(|i| {
            let line = format!(
                "{}  {}  {}  {}",
                pad(i.display_name(), name_w),
                pad(&i.id, id_w),
                pad(&i.status, status_w),
                i.platform,
            );
            format!("{}{}", line.trim_end(), history::marker(&i.id, recent))
        })
        .collect()
}

fn filtered(instances: &[Instance], query: &str) -> Vec<usize> {
    (0..instances.len())
        .filter(|&i| instances[i].matches(query))
        .collect()
}

fn select_terminal(instances: &[Instance], labels: &[String]) -> io::Result<usize> {
    let theme = ColorfulTheme::default();
    let mut visible: Vec<usize> = (0..labels.len()).collect();
    loop {
        let mut items: Vec<&str> = visible.iter().map(|&i| labels[i].as_str()).collect();
        items.push("Search instances");
        let selection = Select::with_theme(&theme)
            .with_prompt("Instance (Esc/q to cancel)")
            .items(&items)
            .default(0)
            .interact_opt()
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("instance selection cancelled"))?;
        if selection < visible.len() {
            return Ok(visible[selection]);
        }
        let query: String = Input::with_theme(&theme)
            .with_prompt("Search name, ID or platform (empty to show all)")
            .allow_empty(true)
            .interact_text()
            .map_err(io::Error::other)?;
        // Keep the current list on no match: an empty menu could only be cancelled.
        let matches = filtered(instances, &query);
        if matches.is_empty() {
            eprintln!("No matching instances.");
        } else {
            visible = matches;
        }
    }
}

fn select_piped(
    instances: &[Instance],
    labels: &[String],
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<usize> {
    let mut visible: Vec<usize> = (0..labels.len()).collect();
    loop {
        writeln!(output, "Select instance:")?;
        for (index, &item) in visible.iter().enumerate() {
            writeln!(output, "  {}. {}", index + 1, labels[item])?;
        }
        write!(
            output,
            "Enter number (/text to search, / to reset, q to cancel): "
        )?;
        output.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Err(io::Error::other(
                "no instance selected (end of input); pass --target for non-interactive use",
            ));
        }
        let selection = line.trim();
        if selection == "q" {
            return Err(io::Error::other("instance selection cancelled"));
        }
        if let Some(query) = selection.strip_prefix('/') {
            let matches = filtered(instances, query);
            if matches.is_empty() {
                writeln!(output, "No matching instances.")?;
            } else {
                visible = matches;
            }
            continue;
        }
        match selection.parse::<usize>() {
            Ok(index) if (1..=visible.len()).contains(&index) => return Ok(visible[index - 1]),
            _ => writeln!(output, "Invalid selection. Try again.")?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(id: &str, name: &str, status: &str) -> Instance {
        Instance {
            id: id.into(),
            name: name.into(),
            status: status.into(),
            platform: "Amazon Linux".into(),
        }
    }

    #[test]
    fn parses_text_output_and_sanitizes_fields() {
        let output = "i-0a\tOnline\tAmazon Linux\tip-10-0-0-1\n\
                      mi-0b\tConnectionLost\tUbuntu\tNone\n\
                      --bad\tOnline\tx\ty\n\
                      i-0a\tOnline\tdup\tdup\n";
        let instances = parse_instances(output);
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].name, "ip-10-0-0-1");
        assert_eq!(instances[1].name, "");
        assert_eq!(instances[1].status, "ConnectionLost");
        let names = parse_names("i-0a\tweb\x1b[2J\ni-0b\tNone\ni-0c\tdb\u{202E}1-i\u{200B}\n");
        assert_eq!(names.len(), 2);
        assert_eq!(names["i-0a"], "web?[2J");
        assert_eq!(names["i-0c"], "db?1-i?");
    }

    #[test]
    fn labels_align_wide_names_and_mark_actual_last_used() {
        let instances = [
            instance("i-0aaa", "web", "Online"),
            instance("i-0bbbbb", "本番DB", "Online"),
            instance("i-0c", "", "ConnectionLost"),
        ];
        // i-gone was used last but no longer exists: nothing is labelled last used.
        let recent = ["i-gone".into(), "i-0aaa".into()];
        let labels = labels(&instances, &recent);
        assert_eq!(
            labels[0],
            "web     i-0aaa    Online          Amazon Linux  (recent)"
        );
        assert_eq!(labels[1], "本番DB  i-0bbbbb  Online          Amazon Linux");
        assert_eq!(labels[2], "-       i-0c      ConnectionLost  Amazon Linux");
        let labels = super::labels(&instances, &["i-0c".into()]);
        assert!(labels[2].ends_with("Amazon Linux  (last used)"));
    }

    #[test]
    fn search_ignores_markers_and_padding() {
        let instances = [
            instance("i-1", "web", "Online"),
            instance("i-2", "db", "Online"),
        ];
        assert_eq!(filtered(&instances, "DB"), [1]);
        assert_eq!(filtered(&instances, "linux"), [0, 1]);
        assert!(filtered(&instances, "recent").is_empty());
        assert!(filtered(&instances, "Online").is_empty());
    }

    #[test]
    fn piped_menu_searches_retries_and_cancels() {
        let instances = [
            instance("i-1", "web", "Online"),
            instance("i-2", "db", "Online"),
        ];
        let labels = labels(&instances, &[]);
        let mut output = Vec::new();
        let index = select_piped(
            &instances,
            &labels,
            &mut &b"x\n9\n/DB\n/nothing\n1\n"[..],
            &mut output,
        );
        assert_eq!(index.unwrap(), 1);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Invalid selection"));
        assert!(output.contains("No matching instances"));
        for input in [&b"q\n"[..], &b""[..]] {
            assert!(select_piped(&instances, &labels, &mut &input[..], &mut Vec::new()).is_err());
        }
    }

    #[test]
    fn targets_cannot_look_like_options() {
        for target in ["i-0123456789abcdef0", "mi-0123", "ecs:cluster_task_1.a"] {
            assert!(valid_target(target), "{target}");
        }
        for target in ["", "-i", "--profile", "i 1", "i-1\n", "i-1;rm"] {
            assert!(!valid_target(target), "{target:?}");
        }
    }

    #[test]
    fn detects_instance_ids_passed_as_profiles() {
        for value in ["i-0123456789abcdef0", "i-0123abcd", "mi-0123456789abcdef0"] {
            assert!(looks_like_instance_id(value), "{value}");
        }
        for value in ["dev", "i-dev", "prod-i-0123abcd", "i-0123"] {
            assert!(!looks_like_instance_id(value), "{value}");
        }
    }
}
