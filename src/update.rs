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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_an_argument_not_shell_code() {
        let command = installer_command(Some("$(touch unexpected)"));
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args[4], "$(touch unexpected)");
        assert!(!INSTALLER.contains("raw.githubusercontent.com"));
    }
}
