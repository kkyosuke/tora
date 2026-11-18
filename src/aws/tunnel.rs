use super::{Settings, history, ssm};
use dialoguer::{Input, Select, theme::ColorfulTheme};
use std::{
    fmt,
    io::{self, BufRead, IsTerminal, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener},
};

const DOCUMENT: &str = "AWS-StartPortForwardingSessionToRemoteHost";

/// A port forwarding destination as seen from the instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Remote {
    host: String,
    port: u16,
}

impl fmt::Display for Remote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

/// A recently used destination: the instance and the remote host behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Tunnel {
    target: String,
    remote: Remote,
}

impl Tunnel {
    /// History value, without the profile: `<target>\t<host>:<port>`.
    fn entry(&self) -> String {
        format!("{}\t{}", self.target, self.remote)
    }

    fn label(&self, recent: &[String]) -> String {
        format!(
            "{}  via {}{}",
            self.remote,
            self.target,
            history::marker(&self.entry(), recent)
        )
    }
}

pub(super) fn run(
    profile: Option<String>,
    target: Option<String>,
    remote: Option<String>,
    local: Option<u16>,
    settings: &Settings,
    no_browser: bool,
    use_device_code: bool,
) -> io::Result<()> {
    // Reject anything that could never work before logging in or listing instances.
    if let Some(target) = &target {
        ssm::check_target(target)?;
    }
    let remote = remote
        .as_deref()
        .map(|remote| parse_remote(remote).map_err(io::Error::other))
        .transpose()?;
    if let Some(port) = local {
        local_port(Some(port), port)?;
    }
    ssm::require_plugin()?;
    let profile = super::resolve_profile(profile, settings)?;
    super::ensure_login(&profile, settings, no_browser, use_device_code)?;

    let tunnel = match (target, remote) {
        (Some(target), Some(remote)) => Tunnel { target, remote },
        (None, Some(remote)) => Tunnel {
            target: ssm::select_target(&profile, settings)?,
            remote,
        },
        (target, None) => choose(&profile, target, settings)?,
    };
    let local = local_port(local, tunnel.remote.port)?;
    // Record history only once a session is actually about to start (not on cancel).
    ssm::remember(&profile, &tunnel.target);
    history::TUNNELS.remember_for(&profile, &tunnel.entry());

    eprintln!(
        "Forwarding localhost:{local} -> {} via {} with profile {profile:?}.",
        tunnel.remote, tunnel.target
    );
    eprintln!("Connect to localhost:{local} while the session runs; press Ctrl+C to stop.");
    let parameters = format!(
        r#"{{"host":["{}"],"portNumber":["{}"],"localPortNumber":["{local}"]}}"#,
        tunnel.remote.host, tunnel.remote.port
    );
    ssm::start_session(
        &profile,
        &tunnel.target,
        settings,
        &["--document-name", DOCUMENT, "--parameters", &parameters],
    )
}

/// Picks a recent destination for the profile, or asks for a new remote host and then an
/// instance. With `target`, only destinations through that instance are offered.
fn choose(profile: &str, target: Option<String>, settings: &Settings) -> io::Result<Tunnel> {
    let recent = history::TUNNELS.for_profile(profile);
    let tunnels: Vec<Tunnel> = recent
        .iter()
        .filter_map(|entry| parse_entry(entry))
        .filter(|tunnel| target.as_ref().is_none_or(|t| *t == tunnel.target))
        .collect();
    let labels: Vec<String> = tunnels.iter().map(|t| t.label(&recent)).collect();
    let terminal = io::stdin().is_terminal() && io::stderr().is_terminal();
    let selection = if labels.is_empty() {
        None
    } else if terminal {
        select_terminal(&labels)?
    } else {
        select_piped(&labels, &mut io::stdin().lock(), &mut io::stderr().lock())?
    };
    if let Some(index) = selection {
        return Ok(tunnels[index].clone());
    }
    let remote = if terminal {
        input_terminal()?
    } else {
        input_piped(&mut io::stdin().lock(), &mut io::stderr().lock())?
    };
    let target = match target {
        Some(target) => target,
        None => ssm::select_target(profile, settings)?,
    };
    Ok(Tunnel { target, remote })
}

const NEW: &str = "Enter a new remote host";

/// `Ok(None)` selects a new remote host.
fn select_terminal(labels: &[String]) -> io::Result<Option<usize>> {
    let mut items: Vec<&str> = labels.iter().map(String::as_str).collect();
    items.push(NEW);
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Port forwarding (Esc/q to cancel)")
        .items(&items)
        .default(0)
        .interact_opt()
        .map_err(io::Error::other)?
        .ok_or_else(|| io::Error::other("port forwarding selection cancelled"))?;
    Ok((selection < labels.len()).then_some(selection))
}

fn read_line(input: &mut impl BufRead) -> io::Result<String> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Err(io::Error::other(
            "no remote host selected (end of input); pass --remote for non-interactive use",
        ));
    }
    Ok(line.trim().to_owned())
}

/// `Ok(None)` selects a new remote host.
fn select_piped(
    labels: &[String],
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<Option<usize>> {
    loop {
        writeln!(output, "Select port forwarding:")?;
        for (index, label) in labels.iter().enumerate() {
            writeln!(output, "  {}. {label}", index + 1)?;
        }
        writeln!(output, "  0. {NEW}")?;
        write!(output, "Enter number (0 for new, q to cancel): ")?;
        output.flush()?;
        let selection = read_line(input)?;
        if selection == "q" {
            return Err(io::Error::other("port forwarding selection cancelled"));
        }
        match selection.parse::<usize>() {
            Ok(0) => return Ok(None),
            Ok(index) if index <= labels.len() => return Ok(Some(index - 1)),
            _ => writeln!(output, "Invalid selection. Try again.")?,
        }
    }
}

fn input_terminal() -> io::Result<Remote> {
    let remote: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Remote host:port (e.g. mydb.example.com:5432)")
        .validate_with(|remote: &String| parse_remote(remote).map(drop))
        .interact_text()
        .map_err(io::Error::other)?;
    parse_remote(&remote).map_err(io::Error::other)
}

fn input_piped(input: &mut impl BufRead, output: &mut impl Write) -> io::Result<Remote> {
    loop {
        write!(output, "Remote host:port (e.g. mydb.example.com:5432): ")?;
        output.flush()?;
        match parse_remote(&read_line(input)?) {
            Ok(remote) => return Ok(remote),
            Err(error) => writeln!(output, "{error}. Try again.")?,
        }
    }
}

/// Parses `HOST:PORT`. Hosts are DNS names or IPv4 addresses; the first character must be
/// alphanumeric so a host can never be read as an option, and only `-`, `.` and `_` are
/// allowed besides letters and digits, which also rules out control and invisible characters.
pub(super) fn parse_remote(value: &str) -> Result<Remote, String> {
    let (host, port) = value.rsplit_once(':').ok_or_else(|| {
        format!("invalid remote {value:?}; expected HOST:PORT such as mydb.example.com:5432")
    })?;
    let valid_host = host.len() <= 253
        && host.starts_with(|c: char| c.is_ascii_alphanumeric())
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'));
    if !valid_host {
        return Err(format!(
            "invalid remote host in {value:?}; expected a host name or IPv4 address"
        ));
    }
    let port = (!port.is_empty() && port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit()))
        .then(|| port.parse::<u16>().ok())
        .flatten()
        .filter(|&port| port != 0)
        .ok_or_else(|| format!("invalid remote port in {value:?}; expected 1-65535"))?;
    Ok(Remote {
        host: host.to_owned(),
        port,
    })
}

/// Parses a history value, `<target>\t<host>:<port>`.
pub(super) fn parse_entry(value: &str) -> Option<Tunnel> {
    let (target, remote) = value.split_once('\t')?;
    Some(Tunnel {
        target: ssm::valid_target(target).then(|| target.to_owned())?,
        remote: parse_remote(remote).ok()?,
    })
}

/// Whether a local listener on `port` would receive the clients' connections. Every
/// loopback and wildcard address is checked: with SO_REUSEADDR (set by Rust on Unix),
/// macOS lets 127.0.0.1 be bound while another process (e.g. a Docker published port)
/// holds 0.0.0.0 or [::], and clients trying `localhost` as ::1 would reach that process.
fn available(port: u16) -> io::Result<()> {
    let addresses: [IpAddr; 4] = [
        Ipv4Addr::LOCALHOST.into(),
        Ipv4Addr::UNSPECIFIED.into(),
        Ipv6Addr::LOCALHOST.into(),
        Ipv6Addr::UNSPECIFIED.into(),
    ];
    for address in addresses {
        match TcpListener::bind((address, port)) {
            Ok(_) => {}
            // Hosts without IPv6 cannot bind IPv6 addresses at all.
            Err(error)
                if address.is_ipv6()
                    && matches!(
                        error.kind(),
                        io::ErrorKind::AddrNotAvailable | io::ErrorKind::Unsupported
                    ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn unavailable(port: u16, error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::AddrInUse {
        format!("localhost:{port} is already in use")
    } else {
        format!("cannot listen on localhost:{port}: {error}")
    }
}

/// An explicit port must be free. Otherwise the remote port is used when it is free, and a
/// port chosen by the OS when it is not (e.g. a local database already listens there, or a
/// privileged port cannot be opened).
fn local_port(requested: Option<u16>, remote: u16) -> io::Result<u16> {
    if let Some(port) = requested {
        return available(port).map(|()| port).map_err(|error| {
            io::Error::other(format!(
                "{}; pass another --local port or omit it",
                unavailable(port, &error)
            ))
        });
    }
    let reason = match available(remote) {
        Ok(()) => return Ok(remote),
        Err(error) => unavailable(remote, &error),
    };
    // An ephemeral port free on 127.0.0.1 can still be taken on another address; retry.
    for _ in 0..10 {
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?
            .local_addr()?
            .port();
        if available(port).is_ok() {
            eprintln!("{reason}; using localhost:{port} instead.");
            return Ok(port);
        }
    }
    Err(io::Error::other(format!(
        "{reason}, and no free local port was found; pass --local"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tunnel(target: &str, remote: &str) -> Tunnel {
        Tunnel {
            target: target.into(),
            remote: parse_remote(remote).unwrap(),
        }
    }

    #[test]
    fn parses_host_and_port() {
        let remote = parse_remote("mydb.cluster-x.ap-northeast-1.rds.amazonaws.com:5432").unwrap();
        assert_eq!(remote.port, 5432);
        assert_eq!(
            remote.to_string(),
            "mydb.cluster-x.ap-northeast-1.rds.amazonaws.com:5432"
        );
        assert_eq!(parse_remote("10.0.0.1:1").unwrap().port, 1);
        assert_eq!(parse_remote("cache_1:65535").unwrap().port, 65535);
    }

    #[test]
    fn rejects_options_control_characters_and_bad_ports() {
        for value in [
            "",
            "db",
            ":5432",
            "-db:5432",
            "--profile:1",
            ".db:1",
            "db host:1",
            "db\n:1",
            "db\u{1b}:1",
            "d\u{202E}b:1",
            "db,x:1",
            "db\":1",
            "::1:5432",
            "[::1]:5432",
            "ｄｂ:1",
        ] {
            assert!(parse_remote(value).is_err(), "{value:?}");
        }
        assert!(
            parse_remote("-db:1")
                .unwrap_err()
                .contains("invalid remote host")
        );
        assert!(
            parse_remote("db")
                .unwrap_err()
                .contains("expected HOST:PORT")
        );
        for value in [
            "db:",
            "db:0",
            "db:65536",
            "db:-1",
            "db:+1",
            "db:1x",
            "db:000001",
            "db: 1",
        ] {
            assert!(
                parse_remote(value)
                    .unwrap_err()
                    .contains("invalid remote port"),
                "{value:?}"
            );
        }
        assert!(parse_remote(&format!("{}:1", "a".repeat(254))).is_err());
    }

    #[test]
    fn history_entries_need_valid_target_and_remote() {
        assert_eq!(
            parse_entry("i-0aaa\tdb:5432"),
            Some(tunnel("i-0aaa", "db:5432"))
        );
        for value in ["i-0aaa", "-i\tdb:1", "i-0aaa\tdb", "i-0aaa\tdb:1\textra"] {
            assert!(parse_entry(value).is_none(), "{value:?}");
        }
    }

    #[test]
    fn labels_mark_only_the_actual_last_used_entry() {
        let recent = ["i-gone\tdb:1".into(), "i-0aaa\tdb:5432".into()];
        assert_eq!(
            tunnel("i-0aaa", "db:5432").label(&recent),
            "db:5432  via i-0aaa  (recent)"
        );
        assert_eq!(
            tunnel("i-gone", "db:1").label(&recent),
            "db:1  via i-gone  (last used)"
        );
        assert_eq!(tunnel("i-1", "db:1").label(&recent), "db:1  via i-1");
    }

    #[test]
    fn piped_menu_selects_new_retries_and_cancels() {
        let labels = ["a".to_owned(), "b".to_owned()];
        let mut output = Vec::new();
        let selection = select_piped(&labels, &mut &b"x\n3\n2\n"[..], &mut output);
        assert_eq!(selection.unwrap(), Some(1));
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("Invalid selection")
        );
        assert_eq!(
            select_piped(&labels, &mut &b"0\n"[..], &mut Vec::new()).unwrap(),
            None
        );
        for input in [&b"q\n"[..], &b""[..]] {
            assert!(select_piped(&labels, &mut &input[..], &mut Vec::new()).is_err());
        }
    }

    #[test]
    fn piped_input_retries_invalid_remote() {
        let mut output = Vec::new();
        let remote = input_piped(&mut &b"db\n-db:1\ndb:5432\n"[..], &mut output).unwrap();
        assert_eq!(remote.to_string(), "db:5432");
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("invalid remote \"db\""), "{output}");
        assert!(
            output.contains("invalid remote host in \"-db:1\""),
            "{output}"
        );
        assert!(input_piped(&mut &b""[..], &mut Vec::new()).is_err());
    }

    #[test]
    fn local_port_prefers_remote_port_and_falls_back_when_busy() {
        let busy = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = busy.local_addr().unwrap().port();
        let other = local_port(None, port).unwrap();
        assert_ne!(other, port);
        assert!(
            local_port(Some(port), port)
                .unwrap_err()
                .to_string()
                .contains("in use")
        );
        drop(busy);
        assert_eq!(local_port(None, port).unwrap(), port);
        assert_eq!(local_port(Some(port), 1).unwrap(), port);
        // A wildcard listener (e.g. a Docker published port) also makes the port busy.
        let wildcard = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        let port = wildcard.local_addr().unwrap().port();
        assert_ne!(local_port(None, port).unwrap(), port);
        assert!(local_port(Some(port), port).is_err());
    }
}
