#![cfg(unix)]

use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/aws-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&path).unwrap();
        let fixture = Self(path);
        fixture.script(
            "aws",
            r#"#!/bin/sh
printf '%s\n' "$@" >> "$CALLS"
if [ "$1" = configure ]; then
  printf '%s' "$PROFILES"
  exit "${LIST_STATUS:-0}"
fi
printf 'login output: $(touch injected)\n'
printf 'login profile: %s\n' "$AWS_PROFILE" >&2
exit "${LOGIN_STATUS:-0}"
"#,
        );
        fixture.script("shell", r#"#!/bin/sh
printf 'shell:%s:%s\n' "$1" "$AWS_PROFILE"
printf 'credentials:%s:%s:%s:%s:%s\n' "${AWS_ACCESS_KEY_ID-unset}" "${AWS_SECRET_ACCESS_KEY-unset}" "${AWS_SESSION_TOKEN-unset}" "${AWS_SECURITY_TOKEN-unset}" "${AWS_DEFAULT_PROFILE-unset}"
printf 'config:%s:%s\n' "$AWS_CONFIG_FILE" "$AWS_SHARED_CREDENTIALS_FILE"
exit "${SHELL_STATUS:-0}"
"#);
        fixture
    }

    fn script(&self, name: &str, body: &str) {
        let path = self.0.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tora"));
        command
            .arg("aws")
            .args(args)
            .current_dir(&self.0)
            .env("PATH", &self.0)
            .env("TORA_HOME", self.0.join("tora"))
            .env("SHELL", self.0.join("shell"))
            .env("CALLS", self.0.join("calls"))
            .env("PROFILES", "default\ndev\ndefault\n")
            .env("AWS_PROFILE", "previous")
            .env("AWS_CONFIG_FILE", self.0.join("config"))
            .env("AWS_SHARED_CREDENTIALS_FILE", self.0.join("credentials"))
            .env("AWS_ACCESS_KEY_ID", "old-key")
            .env("AWS_SECRET_ACCESS_KEY", "old-secret")
            .env("AWS_SESSION_TOKEN", "old-token")
            .env("AWS_SECURITY_TOKEN", "old-token")
            .env("AWS_DEFAULT_PROFILE", "old-default")
            .env_remove("LIST_STATUS")
            .env_remove("LOGIN_STATUS")
            .env_remove("SHELL_STATUS");
        command
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.0.join("calls")).unwrap_or_default()
    }

    fn input(&self, args: &[&str], input: &str) -> Output {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn lists_profiles_without_duplicates_in_original_order() {
    let f = Fixture::new();
    let output = f.command(&["profiles"]).output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"default\ndev\n");
    assert_eq!(f.calls(), "configure\nlist-profiles\n");
}

#[test]
fn explicit_profile_skips_discovery_and_opens_authenticated_shell() {
    let f = Fixture::new();
    let output = f
        .command(&["sso", "custom", "--no-browser", "--use-device-code"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let out = String::from_utf8(output.stdout).unwrap();
    assert!(out.contains("shell:-i:custom"));
    assert!(out.contains("credentials:unset:unset:unset:unset:unset"));
    assert!(out.contains(&format!(
        "config:{}:{}",
        f.0.join("config").display(),
        f.0.join("credentials").display()
    )));
    assert_eq!(
        f.calls(),
        "sso\nlogin\n--profile\ncustom\n--no-browser\n--use-device-code\n"
    );
}

#[test]
fn login_failure_never_opens_shell_or_exports() {
    for extra in [vec![], vec!["--export"]] {
        let f = Fixture::new();
        let mut args = vec!["sso", "dev"];
        args.extend(extra);
        let output = f.command(&args).env("LOGIN_STATUS", "3").output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("aws sso login failed"));
    }
}

#[test]
fn export_is_evaluable_and_cannot_execute_profile_or_login_output() {
    let f = Fixture::new();
    let profile = "a'b $(touch injected); `touch injected`";
    let output = f.command(&["sso", profile, "--export"]).output().unwrap();
    assert!(output.status.success());
    let script = String::from_utf8(output.stdout).unwrap();
    assert!(!script.contains("login output"));
    let evaluated = Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("{script}\nprintf '%s' \"$AWS_PROFILE\""))
        .current_dir(&f.0)
        .output()
        .unwrap();
    assert!(evaluated.status.success());
    assert_eq!(String::from_utf8(evaluated.stdout).unwrap(), profile);
    assert!(!f.0.join("injected").exists());
}

#[test]
fn no_shell_only_logs_in() {
    let f = Fixture::new();
    let output = f.command(&["sso", "dev", "--no-shell"]).output().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("SSO login completed"));
}

#[test]
fn piped_menu_selects_profile() {
    let f = Fixture::new();
    let output = f.input(&["sso", "--export"], "2\n");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("export AWS_PROFILE='dev'"));
}

#[test]
fn empty_discovery_accepts_manual_profile() {
    let f = Fixture::new();
    let mut command = f.command(&["sso", "--export"]);
    command
        .env("PROFILES", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"manual\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("export AWS_PROFILE='manual'"));
}

#[test]
fn cancelled_menu_and_eof_do_not_log_in() {
    for input in ["q\n", ""] {
        let f = Fixture::new();
        let output = f.input(&["sso"], input);
        assert!(!output.status.success());
        assert_eq!(f.calls(), "configure\nlist-profiles\n");
    }
}

#[test]
fn missing_aws_has_actionable_error() {
    let f = Fixture::new();
    fs::remove_file(f.0.join("aws")).unwrap();
    let output = f.command(&["sso", "dev"]).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("install/configure AWS CLI v2"));
    assert!(output.stdout.is_empty());
}

#[test]
fn discovery_failure_does_not_attempt_login() {
    let f = Fixture::new();
    let output = f
        .command(&["sso"])
        .env("LIST_STATUS", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(f.calls(), "configure\nlist-profiles\n");
    assert!(output.stdout.is_empty());
}

#[test]
fn shell_exit_status_is_preserved() {
    let f = Fixture::new();
    let output = f
        .command(&["sso", "dev"])
        .env("SHELL_STATUS", "7")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7));
}

#[test]
fn control_characters_and_empty_profiles_are_rejected_before_login() {
    for profile in ["", "\n", "dev\x1b[2J"] {
        let f = Fixture::new();
        let output = f.command(&["sso", profile]).output().unwrap();
        assert!(!output.status.success());
        assert!(f.calls().is_empty());
    }
}

#[test]
fn export_and_no_shell_are_mutually_exclusive() {
    let f = Fixture::new();
    assert!(
        !f.command(&["sso", "dev", "--export", "--no-shell"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(f.calls().is_empty());
}

#[test]
fn exec_sets_profile_clears_credentials_and_preserves_exit_status() {
    let f = Fixture::new();
    let output = f
        .command(&["exec", "dev", "--", "shell", "child-argument"])
        .env("SHELL_STATUS", "23")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(23));
    let out = String::from_utf8(output.stdout).unwrap();
    assert!(out.contains("shell:child-argument:dev"));
    assert!(out.contains("credentials:unset:unset:unset:unset:unset"));
    assert!(out.contains(&format!(
        "config:{}:{}",
        f.0.join("config").display(),
        f.0.join("credentials").display()
    )));
    assert!(
        f.calls().is_empty(),
        "exec must not discover or log in for an explicit profile"
    );
}

#[test]
fn exec_passes_arguments_literally_and_inherits_streams() {
    let f = Fixture::new();
    f.script("child", "#!/bin/sh\nprintf '%s\\n' \"$@\"\nIFS= read -r line\nprintf 'input:%s\\n' \"$line\"\nprintf 'child stderr\\n' >&2\n");
    let output = f.input(
        &[
            "exec",
            "dev",
            "--",
            "child",
            "--help",
            "a b",
            "",
            "$(touch injected); `touch injected`",
        ],
        "hello\n",
    );
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "--help\na b\n\n$(touch injected); `touch injected`\ninput:hello\n"
    );
    assert_eq!(output.stderr, b"child stderr\n");
    assert!(!f.0.join("injected").exists());
}

#[test]
fn exec_preserves_signal_termination() {
    use std::os::unix::process::ExitStatusExt;
    let f = Fixture::new();
    let output = f
        .command(&["exec", "dev", "--", "/bin/sh", "-c", "kill -TERM $$"])
        .output()
        .unwrap();
    assert_eq!(output.status.signal(), Some(15));
}

#[test]
fn exec_requires_command_and_valid_profile() {
    for args in [
        vec!["exec", "dev"],
        vec!["exec", "dev", "--"],
        vec!["exec", "", "--", "shell"],
        vec!["exec", "bad\nprofile", "--", "shell"],
    ] {
        let f = Fixture::new();
        let output = f.command(&args).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(f.calls().is_empty());
        assert!(!f.0.join("tora/aws-profile-history").exists());
    }
}

#[test]
fn exec_missing_program_reports_failure() {
    let f = Fixture::new();
    let output = f
        .command(&["exec", "dev", "--", "missing-program"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!output.stderr.is_empty());
    assert!(f.calls().is_empty());
}

#[test]
fn exec_can_select_search_or_cancel_without_logging_in() {
    let f = Fixture::new();
    let output = f.input(&["exec", "--", "shell", "arg"], "/DEV\n1\n");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("shell:arg:dev"));
    assert_eq!(f.calls(), "configure\nlist-profiles\n");
    let f = Fixture::new();
    let output = f.input(&["exec", "--", "shell"], "q\n");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!f.0.join("tora/aws-profile-history").exists());
}

#[test]
fn history_is_shared_deduplicated_and_does_not_change_profiles_output() {
    let f = Fixture::new();
    assert!(
        f.command(&["sso", "dev", "--no-shell"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let output = f.input(&["exec", "--", "shell", "arg"], "1\n");
    assert!(String::from_utf8_lossy(&output.stdout).contains("shell:arg:dev"));
    assert!(
        f.command(&["exec", "default", "--", "shell"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let history = f.0.join("tora/aws-profile-history");
    assert_eq!(fs::read_to_string(&history).unwrap(), "default\ndev\n");
    assert_eq!(
        fs::metadata(history).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        f.command(&["profiles"]).output().unwrap().stdout,
        b"default\ndev\n"
    );
    let output = f.input(&["sso", "--export"], "1\n");
    assert!(String::from_utf8_lossy(&output.stdout).contains("export AWS_PROFILE='default'"));
}

#[test]
fn failed_login_does_not_change_history() {
    let f = Fixture::new();
    assert!(
        f.command(&["sso", "dev", "--no-shell"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let output = f
        .command(&["sso", "default", "--export"])
        .env("LOGIN_STATUS", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(f.0.join("tora/aws-profile-history")).unwrap(),
        "dev\n"
    );
}

#[test]
fn corrupt_and_stale_history_do_not_break_selection() {
    for content in [
        b"\xff\xfe".as_slice(),
        b"deleted\n\x1b[2J\ndev\ndev\n",
        b"\n",
    ] {
        let f = Fixture::new();
        fs::create_dir(f.0.join("tora")).unwrap();
        fs::write(f.0.join("tora/aws-profile-history"), content).unwrap();
        let output = f.input(&["sso", "--export"], "/dev\n1\n");
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("export AWS_PROFILE='dev'"));
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(!err.contains("deleted"));
        assert!(!err.contains("\x1b[2J"));
    }
}

#[test]
fn history_is_bounded_and_save_failure_is_nonfatal() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("tora")).unwrap();
    let history = f.0.join("tora/aws-profile-history");
    fs::write(
        &history,
        (0..20)
            .map(|i| format!("profile-{i}\n"))
            .collect::<String>(),
    )
    .unwrap();
    assert!(
        f.command(&["sso", "dev", "--no-shell"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let saved = fs::read_to_string(&history).unwrap();
    assert_eq!(saved.lines().count(), 20);
    assert!(saved.starts_with("dev\nprofile-0\n"));
    assert!(!saved.contains("profile-19\n"));
    fs::remove_file(&history).unwrap();
    fs::create_dir(&history).unwrap();
    let output = f.command(&["sso", "dev", "--export"]).output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("export AWS_PROFILE='dev'"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not save"));
}

#[test]
fn search_with_no_matches_can_reset_or_enter_custom_profile() {
    let f = Fixture::new();
    let output = f.input(&["sso", "--export"], "/missing\n1\n/\n2\n");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("export AWS_PROFILE='dev'"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("No matching profiles"));
    let output = f.input(&["sso", "--export"], "/missing\n0\ncustom\n");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("export AWS_PROFILE='custom'"));
}
