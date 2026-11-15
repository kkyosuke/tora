#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
};

struct Fixture {
    root: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/helper-tests");
        fs::create_dir_all(&cache).unwrap();
        let f = Self {
            root: tempfile::tempdir_in(cache).unwrap(),
        };
        fs::create_dir_all(f.path("tools")).unwrap();
        f.script("aws", r#"#!/bin/sh
printf '%s\n' "$@" >> "$HOME/calls"
printf 'profile=%s key=%s region=%s default_region=%s\n' "$AWS_PROFILE" "$AWS_ACCESS_KEY_ID" "$AWS_REGION" "$AWS_DEFAULT_REGION" >> "$HOME/environment"
if [ "$1" = --version ]; then printf '%s\n' "${TEST_AWS_VERSION:-aws-cli/2.0.0}"; exit 0; fi
if [ "$1" = --profile ]; then shift 2; fi
case "$1 $2" in
  'sts get-caller-identity')
    if [ -n "$TEST_AUTH_ERROR" ]; then printf '%s\n' "$TEST_AUTH_ERROR" >&2; exit 1; fi
    printf '123456789012\tarn:aws:sts::123456789012:assumed-role/Developer/session\tUSER:session\n';;
  'configure get') printf '%s\n' "${TEST_REGION:-ap-northeast-1}"; exit "${TEST_REGION_STATUS:-0}";;
  'configure list-profiles') printf 'default\ndev\n';;
  'sso logout') exit "${TEST_LOGOUT_STATUS:-0}";;
  *) printf 'unexpected AWS command\n' >&2; exit 7;;
esac
"#);
        f.script(
            "curl",
            r#"#!/bin/sh
printf '%s\n' "$@" > "$HOME/curl-calls"
printf '%s' "${TEST_RELEASE_URL:-https://github.com/KKyosuke/tora/releases/tag/v9.0.0}"
exit "${TEST_CURL_STATUS:-0}"
"#,
        );
        f
    }
    fn path(&self, path: &str) -> PathBuf {
        self.root.path().join(path)
    }
    fn script(&self, name: &str, text: &str) {
        let path = self.path(&format!("tools/{name}"));
        fs::write(&path, text).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_tora"));
        c.env_clear()
            .env("HOME", self.root.path())
            .env("TORA_HOME", self.path("tora"))
            .env("ZDOTDIR", self.root.path())
            .env("SHELL", "/bin/bash")
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.path("tools").display()),
            )
            .current_dir(self.root.path())
            .args(args);
        c
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn config(&self, key: &str, value: &str) {
        self.ok(&["config", "set", key, value]);
    }
}

#[test]
fn config_round_trip_is_private_and_unset_is_idempotent() {
    let f = Fixture::new();
    assert!(f.ok(&["config"]).contains("No defaults configured"));
    assert!(!f.path("tora").exists());
    assert!(!f.run(&["config", "get", "aws.profile"]).status.success());
    f.config("aws.profile", "dev team=one");
    f.config("aws.region", "ap-northeast-1");
    assert_eq!(f.ok(&["config", "get", "aws.profile"]), "dev team=one\n");
    assert_eq!(f.ok(&["config", "get", "aws.region"]), "ap-northeast-1\n");
    assert_eq!(
        fs::metadata(f.path("tora/config"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    f.ok(&["config", "unset", "aws.profile"]);
    f.ok(&["config", "unset", "aws.profile"]);
    assert_eq!(f.ok(&["config", "get", "aws.region"]), "ap-northeast-1\n");
    assert!(!f.path("tora/config.lock").exists());
}

#[test]
fn invalid_config_writes_preserve_existing_file() {
    let f = Fixture::new();
    f.config("aws.profile", "dev");
    let before = fs::read(f.path("tora/config")).unwrap();
    for args in [
        vec!["config", "set", "aws.profile", "bad\nprofile"],
        vec!["config", "set", "aws.profile", ""],
        vec!["config", "set", "aws.region", "bad region"],
        vec!["config", "set", "aws.access_key", "secret"],
    ] {
        assert!(!f.run(&args).status.success());
        assert_eq!(fs::read(f.path("tora/config")).unwrap(), before);
    }
    fs::create_dir(f.path("tora/config.lock")).unwrap();
    assert!(
        !f.run(&["config", "set", "aws.profile", "prod"])
            .status
            .success()
    );
    assert_eq!(fs::read(f.path("tora/config")).unwrap(), before);
}

#[test]
fn malformed_and_symlinked_config_is_not_overwritten() {
    let f = Fixture::new();
    fs::create_dir(f.path("tora")).unwrap();
    for contents in [
        "aws.profile=dev\naws.profile=prod\n",
        "unknown=value\n",
        "broken",
    ] {
        fs::write(f.path("tora/config"), contents).unwrap();
        assert!(
            !f.run(&["config", "set", "aws.profile", "new"])
                .status
                .success()
        );
        assert_eq!(fs::read_to_string(f.path("tora/config")).unwrap(), contents);
    }
    fs::remove_file(f.path("tora/config")).unwrap();
    std::os::unix::fs::symlink(f.path("missing"), f.path("tora/config")).unwrap();
    assert!(
        !f.run(&["config", "set", "aws.profile", "new"])
            .status
            .success()
    );
    assert!(f.path("tora/config").is_symlink());
}

#[test]
fn saved_defaults_apply_to_exec_and_environment_wins() {
    let f = Fixture::new();
    f.config("aws.profile", "saved");
    f.config("aws.region", "eu-west-1");
    let args = [
        "aws",
        "exec",
        "--",
        "/bin/sh",
        "-c",
        "printf '%s:%s:%s' \"$AWS_PROFILE\" \"$AWS_REGION\" \"$AWS_DEFAULT_REGION\"",
    ];
    assert_eq!(f.ok(&args), "saved:eu-west-1:eu-west-1");
    let out = f
        .command(&args)
        .env("AWS_PROFILE", "env")
        .env("AWS_DEFAULT_REGION", "us-west-2")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"env:us-west-2:us-west-2");
    let out = f
        .command(&[
            "aws",
            "exec",
            "explicit",
            "--",
            "/bin/sh",
            "-c",
            "printf '%s:%s' \"$AWS_PROFILE\" \"$AWS_REGION\"",
        ])
        .env("AWS_PROFILE", "env")
        .env("AWS_REGION", "us-east-2")
        .env("AWS_DEFAULT_REGION", "us-west-2")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"explicit:us-east-2");
}

#[test]
fn sso_uses_saved_defaults_and_export_propagates_region() {
    let f = Fixture::new();
    f.config("aws.profile", "saved");
    f.config("aws.region", "eu-west-1");
    let out = f.ok(&["aws", "sso", "--export"]);
    assert!(out.contains("export AWS_PROFILE='saved'"));
    assert!(out.contains("export AWS_REGION='eu-west-1' AWS_DEFAULT_REGION='eu-west-1'"));
    assert!(
        !fs::read_to_string(f.path("calls"))
            .unwrap()
            .contains("login")
    );
}

#[test]
fn whoami_preserves_ambient_credentials_but_explicit_profile_clears_them() {
    let f = Fixture::new();
    f.config("aws.profile", "saved");
    for (args, expected) in [
        (vec!["aws", "whoami"], "profile=saved key=ambient"),
        (vec!["aws", "whoami", "dev"], "profile=dev key= region="),
    ] {
        fs::write(f.path("environment"), "").unwrap();
        let out = f
            .command(&args)
            .env("AWS_ACCESS_KEY_ID", "ambient")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("Account: 123456789012"));
        assert!(text.contains("assumed-role/Developer/session"));
        assert!(text.contains("Region: ap-northeast-1"));
        assert!(!text.contains("ambient"));
        assert!(
            fs::read_to_string(f.path("environment"))
                .unwrap()
                .contains(expected)
        );
    }
}

#[test]
fn whoami_failure_does_not_login_and_missing_region_is_supported() {
    let f = Fixture::new();
    let out = f
        .command(&["aws", "whoami"])
        .env("TEST_AUTH_ERROR", "SSO token expired")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(
        !fs::read_to_string(f.path("calls"))
            .unwrap()
            .contains("login")
    );
    let out = f
        .command(&["aws", "whoami"])
        .env("TEST_REGION_STATUS", "1")
        .env("TEST_REGION", "")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("(not configured)"));
}

#[test]
fn logout_is_global_and_reports_failures() {
    let f = Fixture::new();
    let out = f.run(&["aws", "logout"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("ALL"));
    assert_eq!(
        fs::read_to_string(f.path("calls")).unwrap(),
        "sso\nlogout\n"
    );
    assert!(
        !f.command(&["aws", "logout"])
            .env("TEST_LOGOUT_STATUS", "3")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!f.run(&["aws", "logout", "dev"]).status.success());
}

#[test]
fn check_updates_compares_versions_without_installation_or_downloads() {
    let f = Fixture::new();
    for (version, expected) in [
        ("9.0.0", "Update available"),
        (env!("CARGO_PKG_VERSION"), "Already up to date"),
        ("0.0.0", "newer than"),
    ] {
        let out = f
            .command(&["update", "--check"])
            .env(
                "TEST_RELEASE_URL",
                format!("https://github.com/kkyosuke/tora/releases/tag/v{version}"),
            )
            .env("TORA_VERSION", "v0.0.0")
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains(expected));
        assert!(!f.path("tora").exists());
        assert!(
            !fs::read_to_string(f.path("curl-calls"))
                .unwrap()
                .contains("download")
        );
    }
}

#[test]
fn check_updates_rejects_invalid_versions_urls_and_network_errors() {
    let f = Fixture::new();
    for url in [
        "https://github.com/KKyosuke/tora/releases/latest",
        "https://github.com/other/tora/releases/tag/v9.0.0",
        "https://github.com/KKyosuke/tora/releases/tag/v1.0.0-beta",
        "https://github.com/KKyosuke/tora/releases/tag/v01.0.0",
    ] {
        assert!(
            !f.command(&["update", "--check"])
                .env("TEST_RELEASE_URL", url)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    assert!(
        !f.command(&["update", "--check"])
            .env("TEST_CURL_STATUS", "22")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        !f.run(&["update", "--check", "--version", "v1.0.0"])
            .status
            .success()
    );
    assert!(!f.path("tora").exists());
}

#[test]
fn doctor_aggregates_failures_without_installing_or_logging_in() {
    let f = Fixture::new();
    let out = f
        .command(&["doctor"])
        .env("TEST_AUTH_ERROR", "SSO token expired")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for expected in [
        "Installation",
        "PATH",
        "Completion",
        "AWS CLI",
        "AWS authentication / SSO",
        "tora aws sso",
        "tora completion install",
    ] {
        assert!(text.contains(expected), "{text}");
    }
    assert!(!f.path("tora").exists());
    assert!(
        !fs::read_to_string(f.path("calls"))
            .unwrap()
            .contains("login")
    );
}

#[test]
fn doctor_verifies_installed_binary_and_custom_completion_path() {
    let f = Fixture::new();
    f.ok(&[
        "completion",
        "install",
        "--shell",
        "bash",
        "--yes",
        "--path",
        f.path("custom-loader").to_str().unwrap(),
    ]);
    fs::create_dir_all(f.path("tora/bin")).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_tora"), f.path("tora/bin/tora")).unwrap();
    let command = f.command(&[]);
    // Use an installed symlink without copying an executable during parallel process spawns.
    let mut installed = Command::new(f.path("tora/bin/tora"));
    installed
        .env_clear()
        .envs(command.get_envs().filter_map(|(k, v)| v.map(|v| (k, v))))
        .env(
            "PATH",
            format!(
                "{}:{}:/usr/bin:/bin",
                f.path("tora/bin").display(),
                f.path("tools").display()
            ),
        )
        .arg("doctor");
    let out = installed.output().unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("All checks passed"));
    fs::write(f.path("custom-loader"), "broken").unwrap();
    assert!(!installed.output().unwrap().status.success());
}

#[test]
fn empty_tora_home_uses_the_same_default_for_all_commands() {
    let f = Fixture::new();
    let run = |args: &[&str]| f.command(args).env("TORA_HOME", "").output().unwrap();
    assert!(
        run(&["config", "set", "aws.profile", "dev"])
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(f.path(".tora/config")).unwrap(),
        "aws.profile=dev\n"
    );
    assert!(
        run(&["aws", "exec", "dev", "--", "/bin/sh", "-c", "exit 0"])
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(f.path(".tora/aws-profile-history")).unwrap(),
        "dev\n"
    );
    assert!(
        run(&["completion", "install", "--shell", "bash", "--yes"])
            .status
            .success()
    );
    assert!(f.path(".tora/completions/bash").is_file());
    assert!(!f.path("completions").exists());
    fs::create_dir_all(f.path(".tora/bin")).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_tora"), f.path(".tora/bin/tora")).unwrap();
    let output = run(&["update", "--version", "invalid"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("expected a stable version"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("self-update requires"));
    let output = run(&["doctor"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("[OK] Installation:"));
}

#[test]
fn aws_operation_uses_one_settings_snapshot() {
    let f = Fixture::new();
    f.config("aws.profile", "dev");
    let aws = fs::read_to_string(f.path("tools/aws")).unwrap().replace(
        "'sts get-caller-identity')",
        "'sts get-caller-identity')\nprintf 'aws.region=changed-mid-operation\\n' > \"$TORA_HOME/config\"",
    );
    f.script("aws", &aws);
    let text = f.ok(&["aws", "whoami"]);
    assert!(text.contains("Region: ap-northeast-1"));
    assert!(!text.contains("changed-mid-operation"));
    assert!(
        fs::read_to_string(f.path("tora/config"))
            .unwrap()
            .contains("changed-mid-operation")
    );
}
