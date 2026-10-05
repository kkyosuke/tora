use std::{ffi::OsStr, fs, io, path::Path, process::Command};
use tempfile::TempDir;

/// Keep the user's startup configuration and add a badge before each prompt.
/// The shell deletes its private startup directory after reading it; on exec
/// failure, TempDir removes it when this function's caller returns.
pub fn configure(command: &mut Command, shell: &OsStr) -> io::Result<Option<TempDir>> {
    let name = Path::new(shell).file_name().and_then(OsStr::to_str);
    if !matches!(name, Some("zsh" | "bash")) {
        return Ok(None);
    }
    let directory = tempfile::Builder::new().prefix("tora-shell-").tempdir()?;
    command.env("TORA_SHELL_INIT_DIR", directory.path());
    match name {
        Some("zsh") => {
            fs::write(
                directory.path().join(".zshenv"),
                include_str!("../scripts/shell/zshenv"),
            )?;
            let original = std::env::var_os("ZDOTDIR");
            command.env(
                "TORA_ZDOTDIR_SET",
                if original.is_some() { "1" } else { "0" },
            );
            command.env("TORA_ORIGINAL_ZDOTDIR", original.unwrap_or_default());
            command.env("ZDOTDIR", directory.path());
        }
        Some("bash") => {
            let rc = directory.path().join("bashrc");
            fs::write(&rc, include_str!("../scripts/shell/bashrc"))?;
            command.arg("--rcfile").arg(rc);
        }
        _ => unreachable!(),
    }
    Ok(Some(directory))
}
