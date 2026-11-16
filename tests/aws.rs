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
# whoami/status pass an explicit profile before the subcommand.
if [ "$1" = --profile ]; then shift 2; fi
if [ "$1" = configure ] && [ "$2" = get ]; then
  case "$3" in
    sso_session) value=$SSO_SESSION ;;
    sso_start_url) value=$SSO_START_URL ;;
    *) value= ;;
  esac
  [ -n "$value" ] || exit 1
  printf '%s\n' "$value"
  exit 0
fi
if [ "$1" = configure ]; then
  printf '%s' "$PROFILES"
  exit "${LIST_STATUS:-0}"
fi
if [ "$1" = sts ]; then
  if [ -n "$AWS_ACCESS_KEY_ID$AWS_SECRET_ACCESS_KEY$AWS_SESSION_TOKEN$AWS_SECURITY_TOKEN$AWS_DEFAULT_PROFILE" ]; then
    printf 'credential environment was not cleared\n' >&2
    exit 9
  fi
  if [ "${CHECK_STATUS:-1}" = 0 ]; then
    printf '%s\n' "${IDENTITY-123456789012	arn:aws:sts::123456789012:assumed-role/AWSReservedSSO_Admin_0123/alice	AROAEXAMPLE:alice}"
  fi
  printf '%s\n' "${CHECK_ERROR-Error when retrieving token from sso: Token has expired and refresh failed}" >&2
  exit "${CHECK_STATUS:-1}"
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
            .env_remove("AWS_PROFILE")
            .env("AWS_CONFIG_FILE", self.0.join("config"))
            .env("AWS_SHARED_CREDENTIALS_FILE", self.0.join("credentials"))
            .env("AWS_ACCESS_KEY_ID", "old-key")
            .env("AWS_SECRET_ACCESS_KEY", "old-secret")
            .env("AWS_SESSION_TOKEN", "old-token")
            .env("AWS_SECURITY_TOKEN", "old-token")
            .env_remove("AWS_DEFAULT_PROFILE")
            .env_remove("AWS_REGION")
            .env_remove("AWS_DEFAULT_REGION")
            .env_remove("CHECK_STATUS")
            .env_remove("CHECK_ERROR")
            .env_remove("LIST_STATUS")
            .env_remove("LOGIN_STATUS")
            .env_remove("IDENTITY")
            .env_remove("SSO_SESSION")
            .env_remove("SSO_START_URL")
            .env("HOME", &self.0)
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
        "sts\nget-caller-identity\n--profile\ncustom\nsso\nlogin\n--profile\ncustom\n--no-browser\n--use-device-code\n"
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
fn valid_credentials_skip_login_in_all_output_modes() {
    for mode in [None, Some("--export"), Some("--no-shell")] {
        let f = Fixture::new();
        let mut args = vec!["sso", "dev"];
        args.extend(mode);
        let output = f.command(&args).env("CHECK_STATUS", "0").output().unwrap();
        assert!(output.status.success());
        assert_eq!(f.calls(), "sts\nget-caller-identity\n--profile\ndev\n");
        assert_eq!(
            fs::read_to_string(f.0.join("tora/aws-profile-history")).unwrap(),
            "dev\n"
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        match mode {
            None => assert!(stdout.contains("shell:-i:dev")),
            Some("--export") => assert!(stdout.contains("export AWS_PROFILE='dev'")),
            _ => assert!(stdout.is_empty()),
        }
    }
}

#[test]
fn current_profile_is_reused_and_explicit_profile_wins() {
    for (args, expected) in [
        (vec!["sso"], "current"),
        (vec!["sso", "explicit"], "explicit"),
    ] {
        let f = Fixture::new();
        let output = f
            .command(&args)
            .env("AWS_PROFILE", "current")
            .env("AWS_DEFAULT_PROFILE", "fallback")
            .env("CHECK_STATUS", "0")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            f.calls(),
            format!("sts\nget-caller-identity\n--profile\n{expected}\n")
        );
    }
    let f = Fixture::new();
    let output = f
        .command(&["sso"])
        .env("AWS_PROFILE", "")
        .env("AWS_DEFAULT_PROFILE", "fallback")
        .env("CHECK_STATUS", "0")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("shell:-i:fallback"));
}

#[test]
fn select_ignores_current_profile() {
    let f = Fixture::new();
    let mut child = f
        .command(&["sso", "--select"])
        .env("AWS_PROFILE", "current")
        .env("AWS_DEFAULT_PROFILE", "fallback")
        .env("CHECK_STATUS", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"2\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("shell:-i:dev"));
    assert!(f.calls().starts_with("configure\nlist-profiles\n"));
}

#[test]
fn select_conflicts_with_explicit_profile() {
    let f = Fixture::new();
    assert!(
        !f.command(&["sso", "dev", "--select"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(f.calls().is_empty());
}

#[test]
fn missing_and_expired_sso_credentials_trigger_login() {
    for error in [
        "Error loading SSO Token: Token for session does not exist",
        "The SSO session associated with this profile has expired or is otherwise invalid.",
        "An error occurred (ExpiredToken) when calling the GetCallerIdentity operation",
        "An error occurred (UnauthorizedException) when calling the GetRoleCredentials operation",
    ] {
        let f = Fixture::new();
        let output = f
            .command(&["sso", "dev"])
            .env("CHECK_ERROR", error)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(f.calls().contains("sso\nlogin\n"));
    }
}

#[test]
fn unrelated_check_errors_do_not_login_or_export() {
    for error in [
        "Could not connect to the endpoint URL",
        "The config profile (dev) could not be found",
        "An error occurred (AccessDenied) when calling the AssumeRole operation",
    ] {
        for extra in [None, Some("--export")] {
            let f = Fixture::new();
            let mut args = vec!["sso", "dev"];
            args.extend(extra);
            let output = f.command(&args).env("CHECK_ERROR", error).output().unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            assert!(String::from_utf8_lossy(&output.stderr).contains(error));
            assert_eq!(f.calls(), "sts\nget-caller-identity\n--profile\ndev\n");
        }
    }
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
fn exec_login_reuses_valid_credentials_without_logging_in() {
    for args in [
        ["exec", "--login", "dev", "--", "shell", "arg"],
        ["exec", "dev", "--login", "--", "shell", "arg"],
    ] {
        let f = Fixture::new();
        let output = f
            .command(&args)
            .env("CHECK_STATUS", "0")
            .env("SHELL_STATUS", "23")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(23));
        let out = String::from_utf8(output.stdout).unwrap();
        assert!(out.starts_with("shell:arg:dev\n"), "{out}");
        assert!(out.contains("credentials:unset:unset:unset:unset:unset"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("skipping login"));
        assert_eq!(f.calls(), "sts\nget-caller-identity\n--profile\ndev\n");
        assert_eq!(
            fs::read_to_string(f.0.join("tora/aws-profile-history")).unwrap(),
            "dev\n"
        );
    }
}

#[test]
fn exec_login_logs_in_when_expired_and_keeps_stdout_for_the_command() {
    let f = Fixture::new();
    let output = f
        .command(&[
            "exec",
            "--login",
            "--no-browser",
            "--use-device-code",
            "dev",
            "--",
            "shell",
            "arg",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let out = String::from_utf8(output.stdout).unwrap();
    assert!(out.starts_with("shell:arg:dev\n"), "{out}");
    assert!(!out.contains("login output"));
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("login output"));
    assert!(err.contains("SSO login completed"));
    assert_eq!(
        f.calls(),
        "sts\nget-caller-identity\n--profile\ndev\nsso\nlogin\n--profile\ndev\n--no-browser\n--use-device-code\n"
    );
    assert_eq!(
        fs::read_to_string(f.0.join("tora/aws-profile-history")).unwrap(),
        "dev\n"
    );
}

#[test]
fn exec_login_failure_never_runs_the_command() {
    for (name, value, calls) in [
        (
            "LOGIN_STATUS",
            "3",
            "sts\nget-caller-identity\n--profile\ndev\nsso\nlogin\n--profile\ndev\n",
        ),
        (
            "CHECK_ERROR",
            "Could not connect to the endpoint URL",
            "sts\nget-caller-identity\n--profile\ndev\n",
        ),
    ] {
        let f = Fixture::new();
        f.script("child", "#!/bin/sh\ntouch ran\n");
        let output = f
            .command(&["exec", "--login", "dev", "--", "child"])
            .env(name, value)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
        assert_eq!(f.calls(), calls);
        assert!(!f.0.join("ran").exists());
        assert!(!f.0.join("tora/aws-profile-history").exists());
    }
}

#[test]
fn exec_login_options_require_login_and_stay_literal_after_separator() {
    for option in ["--no-browser", "--use-device-code"] {
        let f = Fixture::new();
        let output = f
            .command(&["exec", "dev", option, "--", "shell"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--login"));
        assert!(f.calls().is_empty());
    }
    let f = Fixture::new();
    f.script("child", "#!/bin/sh\nprintf '%s\\n' \"$@\"\n");
    let output = f
        .command(&["exec", "dev", "--", "child", "--login", "--no-browser"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"--login\n--no-browser\n");
    assert!(f.calls().is_empty());
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

const STS_STATUS_CALL: &str = "--profile\ndev\nsts\nget-caller-identity\n--query\n[Account,Arn,UserId]\n--output\ntext\n--cli-connect-timeout\n5\n--cli-read-timeout\n10\n";
const SESSION_CACHE: &str = "a9993e364706816aba3e25717850c26c9cd0d89d"; // SHA-1 of "abc"
const START_URL: &str = "https://example.awsapps.com/start";
const START_URL_CACHE: &str = "e8be5486177c5b5392bd9aa76563515b29358e6e";

fn write_sso_cache(f: &Fixture, name: &str, content: &str) {
    let dir = f.0.join(".aws/sso/cache");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{name}.json")), content).unwrap();
}

#[test]
fn status_shows_identity_and_sso_session_expiry_without_leaking_tokens() {
    let f = Fixture::new();
    write_sso_cache(
        &f,
        SESSION_CACHE,
        r#"{"startUrl": "https://example.awsapps.com/start", "accessToken": "secret-access-token", "expiresAt": "2099-01-01T00:00:00Z", "refreshToken": "secret-refresh-token"}"#,
    );
    // The legacy cache must be ignored when the profile uses an sso-session.
    write_sso_cache(
        &f,
        START_URL_CACHE,
        r#"{"accessToken": "secret-legacy-token", "expiresAt": "2000-01-01T00:00:00Z"}"#,
    );
    let output = f
        .command(&["status", "dev"])
        .env("CHECK_STATUS", "0")
        .env("SSO_SESSION", "abc")
        .env("SSO_START_URL", START_URL)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    // The whoami report, followed by the SSO expiry.
    assert!(stdout.starts_with(
        "Profile: dev\nAccount: 123456789012\n\
         ARN: arn:aws:sts::123456789012:assumed-role/AWSReservedSSO_Admin_0123/alice\n\
         User ID: AROAEXAMPLE:alice\nRegion: (not configured)\n"
    ));
    assert!(stdout.contains("SSO expires: 2099-01-01T00:00:00Z ("));
    assert!(stdout.contains("remaining, refreshed automatically"));
    assert!(!stdout.contains("secret") && !stderr.contains("secret"));
    assert_eq!(
        f.calls(),
        format!(
            "{STS_STATUS_CALL}--profile\ndev\nconfigure\nget\nregion\n\
             --profile\ndev\nconfigure\nget\nsso_session\n"
        )
    );
    assert!(
        !f.0.join("tora/aws-profile-history").exists(),
        "status is read-only"
    );
}

#[test]
fn status_reads_legacy_start_url_cache() {
    let f = Fixture::new();
    write_sso_cache(
        &f,
        START_URL_CACHE,
        r#"{"startUrl": "https://example.awsapps.com/start", "region": "us-east-1", "accessToken": "secret-legacy-token", "expiresAt": "2099-01-01T00:00:00UTC"}"#,
    );
    let output = f
        .command(&["status", "dev"])
        .env("CHECK_STATUS", "0")
        .env("SSO_START_URL", START_URL)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("SSO expires: 2099-01-01T00:00:00UTC ("));
    assert!(stdout.contains(" remaining)\n"));
    assert!(!stdout.contains("secret"));
}

#[test]
fn status_reports_unknown_expiry_without_failing() {
    for (session, cache) in [
        (None, None),
        (Some("abc"), None),
        (Some("abc"), Some("not json")),
        (Some("abc"), Some(r#"{"accessToken": "secret-token"}"#)),
        (
            Some("abc"),
            Some(r#"{"accessToken": "secret-token", "expiresAt": "tomorrow"}"#),
        ),
    ] {
        let f = Fixture::new();
        if let Some(cache) = cache {
            write_sso_cache(&f, SESSION_CACHE, cache);
        }
        let mut command = f.command(&["status", "dev"]);
        command.env("CHECK_STATUS", "0");
        if let Some(session) = session {
            command.env("SSO_SESSION", session);
        }
        let output = command.output().unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("Account: 123456789012\n"));
        assert!(stdout.ends_with("SSO expires: unknown\n"), "{stdout}");
        assert!(!stdout.contains("secret"));
    }
}

#[test]
fn status_fails_without_logging_in_when_unauthenticated() {
    for error in [
        "Error loading SSO Token: Token for session does not exist",
        "Error when retrieving token from sso: Token has expired and refresh failed",
    ] {
        let f = Fixture::new();
        write_sso_cache(
            &f,
            SESSION_CACHE,
            r#"{"accessToken": "secret-token", "expiresAt": "2000-01-01T00:00:00Z"}"#,
        );
        let output = f
            .command(&["status", "dev"])
            .env("CHECK_ERROR", error)
            .env("SSO_SESSION", "abc")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            stdout,
            "Profile: dev\nSSO expires: 2000-01-01T00:00:00Z (expired)\n"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("not logged in or the session has expired"));
        assert!(stderr.contains("`tora aws sso dev`"));
        assert!(!stderr.contains("secret"));
        assert!(!f.calls().contains("login"));
    }
}

#[test]
fn status_reports_unrelated_errors_and_does_not_echo_unsafe_profiles() {
    let f = Fixture::new();
    let output = f
        .command(&["status", "dev"])
        .env("CHECK_ERROR", "Could not connect to the endpoint URL")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Could not connect"));
    assert_eq!(f.calls(), STS_STATUS_CALL);

    let f = Fixture::new();
    let output = f
        .command(&["status", "$(touch injected)"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("run `tora aws sso` to log in"));
}

#[test]
fn status_resolves_profile_like_sso() {
    for (args, expected) in [
        (vec!["status"], "current"),
        (vec!["status", "explicit"], "explicit"),
    ] {
        let f = Fixture::new();
        let output = f
            .command(&args)
            .env("AWS_PROFILE", "current")
            .env("AWS_DEFAULT_PROFILE", "fallback")
            .env("CHECK_STATUS", "0")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stdout).starts_with(&format!("Profile: {expected}\n"))
        );
    }
    let f = Fixture::new();
    let output = f
        .command(&["status"])
        .env("AWS_PROFILE", "")
        .env("AWS_DEFAULT_PROFILE", "fallback")
        .env("CHECK_STATUS", "0")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("Profile: fallback\n"));
    // Without environment variables, the saved default is used before the menu.
    let f = Fixture::new();
    fs::create_dir_all(f.0.join("tora")).unwrap();
    fs::write(f.0.join("tora/config"), "aws.profile=saved\n").unwrap();
    let output = f
        .command(&["status"])
        .env("CHECK_STATUS", "0")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("Profile: saved\n"));
    assert!(!f.calls().contains("list-profiles"));
    let f = Fixture::new();
    let output = f.input(&["status"], "2\n");
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("Profile: dev\n"));
    assert!(f.calls().starts_with("configure\nlist-profiles\n"));
    let f = Fixture::new();
    assert!(!f.input(&["status"], "q\n").status.success());
    assert_eq!(f.calls(), "configure\nlist-profiles\n");
}
