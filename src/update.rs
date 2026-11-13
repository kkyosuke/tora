use std::{io, process::Command};

// Updates use the installer shipped with this exact binary, never a script from main.
const INSTALLER: &str = include_str!("../scripts/install.sh");

fn installer_command(version: Option<&str>) -> Command {
    let mut command = Command::new("bash");
    command.args(["-c", INSTALLER, "tora-installer"]);
    // An ambient TORA_VERSION must not change the meaning of `tora update`.
    command.env_remove("TORA_VERSION");
    if let Some(version) = version {
        command.args(["--version", version]);
    }
    command
}

pub fn run(version: Option<&str>) -> io::Result<()> {
    // Source builds and copies elsewhere must not silently install a different binary.
    let home = std::env::var_os("TORA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".tora")))
        .ok_or_else(|| io::Error::other("HOME or TORA_HOME is required"))?;
    let installed = home.join("bin/tora").canonicalize().ok();
    if installed.as_deref() != Some(std::env::current_exe()?.canonicalize()?.as_path()) {
        return Err(io::Error::other(
            "self-update requires the installed binary at $TORA_HOME/bin/tora (default: ~/.tora/bin/tora); run scripts/install.sh first",
        ));
    }
    let status = installer_command(version).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("installer failed: {status}")))
    }
}

fn stable_version(value: &str) -> io::Result<[u64; 3]> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3
        || parts.iter().any(|p| {
            p.is_empty()
                || (p.len() > 1 && p.starts_with('0'))
                || !p.bytes().all(|b| b.is_ascii_digit())
        })
    {
        return Err(io::Error::other(format!(
            "invalid stable release version: {value}"
        )));
    }
    let mut version = [0; 3];
    for (index, part) in parts.iter().enumerate() {
        version[index] = part.parse().map_err(io::Error::other)?;
    }
    Ok(version)
}

pub fn check() -> io::Result<()> {
    let output = Command::new("curl")
        .args([
            "--disable",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--tlsv1.2",
            "-fsSL",
            "--connect-timeout",
            "5",
            "--max-time",
            "30",
            "-o",
            "/dev/null",
            "-w",
            "%{url_effective}",
            "https://github.com/KKyosuke/tora/releases/latest",
        ])
        .output()
        .map_err(|e| {
            io::Error::other(format!("could not check releases (curl is required): {e}"))
        })?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "release check failed: {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let url = String::from_utf8_lossy(&output.stdout);
    let version = url
        .trim()
        .strip_prefix("https://github.com/")
        .and_then(|path| path.split_once("/releases/tag/v"))
        .filter(|(repo, _)| repo.eq_ignore_ascii_case("kkyosuke/tora"))
        .map(|(_, version)| version)
        .ok_or_else(|| {
            io::Error::other("could not resolve a stable release; check GitHub Releases")
        })?;
    let latest = stable_version(version)?;
    let current = stable_version(env!("CARGO_PKG_VERSION"))?;
    println!("Current: {}\nLatest: {version}", env!("CARGO_PKG_VERSION"));
    match current.cmp(&latest) {
        std::cmp::Ordering::Less => println!("Update available. Run tora update to install."),
        std::cmp::Ordering::Equal => println!("Already up to date."),
        std::cmp::Ordering::Greater => {
            println!("Current version is newer than the latest release.")
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_compared_numerically() {
        assert!(stable_version("0.10.0").unwrap() > stable_version("0.9.99").unwrap());
        assert!(stable_version("1.0.0").unwrap() > stable_version("0.99.99").unwrap());
        for value in ["01.0.0", "1.0.0-rc1", "1.0", "1.0.0.0", "1.0.a"] {
            assert!(stable_version(value).is_err());
        }
    }

    #[test]
    fn version_is_an_argument_not_shell_code() {
        let command = installer_command(Some("$(touch unexpected)"));
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args[4], "$(touch unexpected)");
        assert!(!INSTALLER.contains("raw.githubusercontent.com"));
    }
}
