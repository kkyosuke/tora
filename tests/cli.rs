use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

const BINARY: &str = env!("CARGO_BIN_EXE_tora");
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct InstallHome(PathBuf);

impl InstallHome {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-homes")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(path.join("bin")).unwrap();
        Self(path)
    }

    fn binary(&self) -> PathBuf {
        self.0.join("bin/tora")
    }

    fn install(&self) {
        fs::copy(BINARY, self.binary()).unwrap();
    }
}

impl Drop for InstallHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(BINARY).args(args).output().unwrap()
}

#[test]
fn reports_package_version() {
    let output = run(&["--version"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("tora {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn help_lists_available_commands() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("update"));
    assert!(help.contains("completion"));
}

#[test]
fn invalid_commands_and_arguments_fail() {
    for args in [
        vec!["unknown"],
        vec!["update", "--unknown"],
        vec!["update", "--version"],
        vec!["completion", "unknown-shell"],
    ] {
        let output = run(&args);
        assert!(!output.status.success(), "{args:?}");
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn generates_completion_for_supported_shells() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let output = run(&["completion", shell]);
        assert!(output.status.success(), "{shell}");
        let script = String::from_utf8(output.stdout).unwrap();
        assert!(script.contains("tora"));
        assert!(script.contains("update"));
    }
}

#[test]
fn source_build_cannot_update_a_separate_installation() {
    let home = InstallHome::new();
    home.install();
    let before = fs::read(home.binary()).unwrap();
    let output = Command::new(BINARY)
        .arg("update")
        .env("TORA_HOME", &home.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("self-update requires"));
    assert_eq!(fs::read(home.binary()).unwrap(), before);
}

#[test]
fn missing_installation_gives_actionable_error() {
    let home = InstallHome::new();
    let output = Command::new(BINARY)
        .arg("update")
        .env("TORA_HOME", &home.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("run scripts/install.sh first"));
}

#[cfg(unix)]
#[test]
fn installed_cli_rejects_shell_injection_and_preserves_binary() {
    let home = InstallHome::new();
    home.install();
    let before = fs::read(home.binary()).unwrap();
    let output = Command::new(home.binary())
        .args(["update", "--version", "$(touch injected)"])
        .current_dir(&home.0)
        .env("TORA_HOME", &home.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("expected a stable version"));
    assert!(!home.0.join("injected").exists());
    assert_eq!(fs::read(home.binary()).unwrap(), before);
    assert!(!home.0.join("update.lock").exists());
}
