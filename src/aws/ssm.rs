use super::{history, profile_command, recent_marker};
use dialoguer::{Input, Select, theme::ColorfulTheme};
use std::{
    io::{self, BufRead, IsTerminal, Write},
    path::Path,
    process::Stdio,
};

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

pub(super) fn run(
    profile: Option<String>,
    target: Option<String>,
    no_browser: bool,
    use_device_code: bool,
) -> io::Result<()> {
    if let Some(target) = &target {
        check_target(target)?;
    }
    // Fail before any login or selection when the session could never start.
    if !plugin_installed() {
        return Err(io::Error::other(format!(
            "{PLUGIN} not found in PATH; install the Session Manager plugin for AWS CLI first \
             (https://docs.aws.amazon.com/systems-manager/latest/userguide/session-manager-working-with-install-plugin.html)"
        )));
    }
    let profile = super::resolve_profile(profile)?;
    super::ensure_login(&profile, no_browser, use_device_code)?;
    history::remember(&profile);

    let target = match target {
        Some(target) => target,
        None => {
            let mut instances = list_instances(&profile)?;
            if instances.is_empty() {
                return Err(io::Error::other(format!(
                    "no Session Manager managed instances found for profile {profile:?}; \
                     check the profile's region, or pass --target"
                )));
            }
            let recent = recent_targets(&profile);
            let used = history::sort_by_recent(&mut instances, &recent, |i| &i.id);
            let recent = &recent_ids(&instances[..used]);
            let labels = labels(&instances, recent);
            let index = if io::stdin().is_terminal() && io::stderr().is_terminal() {
                select_terminal(&labels)?
            } else {
                select_piped(&labels, &mut io::stdin().lock(), &mut io::stderr().lock())?
            };
            instances.swap_remove(index).id
        }
    };
    history::INSTANCES.remember(&format!("{profile}\t{target}"));

    eprintln!("Starting Session Manager session on {target} with profile {profile:?}...");
    let mut command = profile_command(&profile);
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

/// Instance IDs used with this profile, most recent first.
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

fn recent_ids(instances: &[Instance]) -> Vec<String> {
    instances.iter().map(|i| i.id.clone()).collect()
}

fn text_query(profile: &str, args: &[&str]) -> io::Result<String> {
    let output = profile_command(profile)
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

fn list_instances(profile: &str) -> io::Result<Vec<Instance>> {
    let output = text_query(
        profile,
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
    let mut names = Vec::new();
    for chunk in ec2_ids.chunks(FILTER_CHUNK) {
        let filter = format!("Name=instance-id,Values={}", chunk.join(","));
        match text_query(
            profile,
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
                eprintln!("tora: could not read EC2 Name tags; showing IDs only\n{error}");
                break;
            }
        }
    }
    for instance in &mut instances {
        if let Some((_, name)) = names.iter().find(|(id, _)| *id == instance.id) {
            instance.name = name.clone();
        }
    }
    // Online instances first, then by name, so the list is stable between runs.
    instances.sort_by(|a, b| {
        (a.status != "Online", &a.name, &a.id).cmp(&(b.status != "Online", &b.name, &b.id))
    });
    Ok(instances)
}

/// AWS CLI text output prints null as `None`; terminal control characters from tags are
/// replaced so names cannot rewrite the menu.
fn field(value: Option<&str>) -> String {
    match value.map(str::trim) {
        None | Some("") | Some("None") => String::new(),
        Some(value) => value
            .chars()
            .map(|c| if c.is_control() { '?' } else { c })
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

fn parse_names(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| {
            let (id, name) = line.split_once('\t')?;
            let name = field(Some(name));
            (!name.is_empty()).then(|| (id.trim().to_owned(), name))
        })
        .collect()
}

fn labels(instances: &[Instance], recent: &[String]) -> Vec<String> {
    let width = |f: fn(&Instance) -> &str| {
        instances
            .iter()
            .map(|i| f(i).chars().count())
            .max()
            .unwrap_or(0)
    };
    let (name_w, id_w, status_w) = (
        width(|i| display_name(i)).max(1),
        width(|i| &i.id),
        width(|i| &i.status),
    );
    instances
        .iter()
        .map(|i| {
            let line = format!(
                "{:<name_w$}  {:<id_w$}  {:<status_w$}  {}",
                display_name(i),
                i.id,
                i.status,
                i.platform,
            );
            format!(
                "{}{}",
                line.trim_end(),
                recent_marker(recent.iter().position(|r| *r == i.id))
            )
        })
        .collect()
}

fn display_name(instance: &Instance) -> &str {
    if instance.name.is_empty() {
        "-"
    } else {
        &instance.name
    }
}

fn filtered(labels: &[String], query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    (0..labels.len())
        .filter(|&i| labels[i].to_lowercase().contains(&query))
        .collect()
}

fn select_terminal(labels: &[String]) -> io::Result<usize> {
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
        let matches = filtered(labels, &query);
        if matches.is_empty() {
            eprintln!("No matching instances.");
        } else {
            visible = matches;
        }
    }
}

fn select_piped(
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
            let matches = filtered(labels, query);
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
        assert_eq!(
            parse_names("i-0a\tweb\x1b[2J\ni-0b\tNone\n"),
            vec![("i-0a".into(), "web?[2J".into())]
        );
    }

    #[test]
    fn labels_align_columns_and_mark_history() {
        let instances = [
            instance("i-0aaa", "web", "Online"),
            instance("i-0bbbbb", "", "Online"),
            instance("i-0c", "batch-worker", "ConnectionLost"),
        ];
        let labels = labels(&instances, &["i-0aaa".into(), "i-0c".into()]);
        assert_eq!(
            labels[0],
            "web           i-0aaa    Online          Amazon Linux  (last used)"
        );
        assert_eq!(
            labels[1],
            "-             i-0bbbbb  Online          Amazon Linux"
        );
        assert!(labels[2].ends_with("Amazon Linux  (recent)"));
    }

    #[test]
    fn piped_menu_searches_retries_and_cancels() {
        let labels = vec!["web i-1".to_owned(), "db i-2".to_owned()];
        let mut output = Vec::new();
        let index = select_piped(&labels, &mut &b"x\n9\n/DB\n/nothing\n1\n"[..], &mut output);
        assert_eq!(index.unwrap(), 1);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Invalid selection"));
        assert!(output.contains("No matching instances"));
        for input in [&b"q\n"[..], &b""[..]] {
            assert!(select_piped(&labels, &mut &input[..], &mut Vec::new()).is_err());
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
}
