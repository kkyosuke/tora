#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    process::{Command, Output},
};

struct Fixture(tempfile::TempDir);

impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/completion-tests");
        fs::create_dir_all(&root).unwrap();
        Self(tempfile::tempdir_in(root).unwrap())
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.path().join(name)
    }

    fn command(&self, shell: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tora"));
        command
            .args(["completion", "install", "--shell", shell, "--yes"])
            .current_dir(self.0.path())
            .env("HOME", self.0.path())
            .env_remove("TORA_HOME")
            .env_remove("ZDOTDIR");
        command
    }

    fn install(&self, shell: &str) -> Output {
        self.command(shell).output().unwrap()
    }
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn creates_and_upserts_without_touching_surrounding_configuration() {
    let f = Fixture::new();
    fs::write(f.path(".zshrc"), "export KEEP=before").unwrap();
    success(&f.install("zsh"));
    let first = fs::read_to_string(f.path(".zshrc")).unwrap();
    assert!(first.starts_with("export KEEP=before\n"));
    let with_tail = format!("{first}export KEEP_AFTER=after\n");
    fs::write(f.path(".zshrc"), &with_tail).unwrap();
    let output = f.install("zsh");
    success(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("already installed"));
    assert_eq!(fs::read_to_string(f.path(".zshrc")).unwrap(), with_tail);
    let custom = f.path("custom path's $(touch injected)/zsh");
    success(
        &f.command("zsh")
            .arg("--path")
            .arg(&custom)
            .output()
            .unwrap(),
    );
    let changed = fs::read_to_string(f.path(".zshrc")).unwrap();
    assert_eq!(changed.matches("# >>> tora completion >>>").count(), 1);
    assert!(changed.ends_with("export KEEP_AFTER=after\n"));
    // The chosen path is remembered when --path is omitted.
    success(&f.install("zsh"));
    assert_eq!(fs::read_to_string(f.path(".zshrc")).unwrap(), changed);
    assert!(!f.path("injected").exists());
}

#[test]
fn respects_zdotdir_tora_home_and_dotfile_symlinks() {
    let f = Fixture::new();
    fs::create_dir(f.path("zsh-config")).unwrap();
    fs::write(f.path("real-rc"), "# my dotfile\n").unwrap();
    symlink(f.path("real-rc"), f.path("zsh-config/.zshrc")).unwrap();
    success(
        &f.command("zsh")
            .env("ZDOTDIR", f.path("zsh-config"))
            .env("TORA_HOME", f.path("tora-home"))
            .output()
            .unwrap(),
    );
    assert!(f.path("zsh-config/.zshrc").is_symlink());
    assert!(
        fs::read_to_string(f.path("real-rc"))
            .unwrap()
            .starts_with("# my dotfile\n")
    );
    assert!(f.path("tora-home/completions/zsh").is_file());
    assert!(!f.path(".zshrc").exists());
}

#[test]
fn manual_or_malformed_configuration_is_never_overwritten() {
    for original in [
        "source <(tora completion zsh)\n",
        "# >>> tora completion >>>\n",
        "# <<< tora completion <<<\n",
    ] {
        let f = Fixture::new();
        fs::write(f.path(".zshrc"), original).unwrap();
        assert!(!f.install("zsh").status.success());
        assert_eq!(fs::read_to_string(f.path(".zshrc")).unwrap(), original);
        assert!(!f.path(".tora").exists());
    }
}

#[test]
fn refuses_unknown_destination_and_broken_symlink_without_changing_rc() {
    for link in [false, true] {
        let f = Fixture::new();
        let destination = f.path("existing");
        if link {
            symlink(f.path("absent"), &destination).unwrap();
        } else {
            fs::write(&destination, "user content").unwrap();
        }
        let output = f
            .command("bash")
            .arg("--path")
            .arg(&destination)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!f.path(".bashrc").exists());
        assert!(!f.path(".bash_profile").exists());
        if !link {
            assert_eq!(fs::read_to_string(destination).unwrap(), "user content");
        }
    }
}

#[test]
fn unattended_install_requires_explicit_consent_and_shell() {
    let f = Fixture::new();
    for args in [
        vec!["completion", "install"],
        vec!["completion", "install", "--yes"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_tora"))
            .args(args)
            .env("HOME", f.0.path())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!f.path(".tora").exists());
    }
}

#[test]
fn bash_uses_first_existing_login_file_and_preserves_it() {
    let f = Fixture::new();
    fs::write(f.path(".profile"), "export PRESERVED=yes\n").unwrap();
    success(&f.install("bash"));
    assert!(!f.path(".bash_profile").exists());
    assert!(
        fs::read_to_string(f.path(".profile"))
            .unwrap()
            .starts_with("export PRESERVED=yes\n")
    );
    assert!(f.path(".bashrc").is_file());
    success(&f.install("bash"));
    assert_eq!(
        fs::read_to_string(f.path(".profile"))
            .unwrap()
            .matches("# >>> tora completion >>>")
            .count(),
        1
    );
}

#[test]
fn actual_shells_load_completions_and_quote_custom_paths() {
    let f = Fixture::new();
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_tora"))
        .parent()
        .unwrap()
        .to_owned();
    for (shell, assertion) in [
        ("bash", "complete -p tora"),
        (
            "zsh",
            "[[ ${_comps[tora]} == _tora_dynamic ]] && functions _tora",
        ),
    ] {
        if Command::new(shell).arg("--version").output().is_err() {
            eprintln!("Skipping {shell}: not installed");
            continue;
        }
        success(
            &f.command(shell)
                .arg("--path")
                .arg(f.path(&format!("quote' $(touch injected)/{shell}")))
                .output()
                .unwrap(),
        );
        let rc = f.path(if shell == "bash" { ".bashrc" } else { ".zshrc" });
        // Do not read the real user's startup files. All generated files and
        // compinit's cache live in this fixture.
        let output = Command::new(shell)
            .args(["-f", "-c", &format!("source \"$TEST_RC\"; {assertion}")])
            .env("TEST_RC", rc)
            .env("HOME", f.0.path())
            .env("ZDOTDIR", f.0.path())
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env_remove("BASH_ENV")
            .current_dir(f.0.path())
            .output()
            .unwrap();
        success(&output);
        assert!(!f.path("injected").exists());
    }
}

#[test]
fn failed_preparation_does_not_modify_existing_startup_files() {
    let f = Fixture::new();
    fs::write(f.path(".bashrc"), "# preserved\n").unwrap();
    fs::create_dir(f.path(".bash_profile")).unwrap();
    assert!(!f.install("bash").status.success());
    assert_eq!(
        fs::read_to_string(f.path(".bashrc")).unwrap(),
        "# preserved\n"
    );
    assert!(!f.path(".tora").exists());
}

#[test]
fn different_shells_cannot_overwrite_each_others_loader() {
    let f = Fixture::new();
    let path = f.path("shared-loader");
    success(&f.command("zsh").arg("--path").arg(&path).output().unwrap());
    let original = fs::read(&path).unwrap();
    let output = f.command("bash").arg("--path").arg(&path).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("different shell"));
    assert_eq!(fs::read(path).unwrap(), original);
    assert!(!f.path(".bashrc").exists());
}

fn fake_aws(f: &Fixture, body: &str) -> String {
    let bin = f.path("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("aws"), format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(bin.join("aws"), fs::Permissions::from_mode(0o755)).unwrap();
    let tora = PathBuf::from(env!("CARGO_BIN_EXE_tora"));
    format!(
        "{}:{}:/usr/bin:/bin",
        bin.display(),
        tora.parent().unwrap().display()
    )
}

fn profile_request(path: &str, words: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tora"))
        .args(["__complete-aws-profiles", "--"])
        .args(words)
        .env("PATH", path)
        .output()
        .unwrap()
}

#[test]
fn profile_request_lists_profiles_only_before_the_command_separator() {
    let f = Fixture::new();
    let path = fake_aws(
        &f,
        r#"[ "$*" = "configure list-profiles" ] && printf 'default\ndev admin\ndefault\n'"#,
    );
    for words in [
        &["aws", "sso"][..],
        &["aws", "exec"],
        &["aws", "sso", "--select"],
    ] {
        let output = profile_request(&path, words);
        success(&output);
        assert_eq!(output.stdout, b"default\ndev admin\n", "{words:?}");
    }
    for words in [
        &["aws", "exec", "--"][..],
        &["aws", "exec", "default", "--"],
        &["aws", "exec", "default", "--", "aws", "sso"],
        &["aws", "sso", "default"],
        &["aws", "profiles"],
        &["aws"],
        &[],
    ] {
        let output = profile_request(&path, words);
        success(&output);
        assert!(output.stdout.is_empty(), "{words:?}");
    }
}

#[test]
fn profile_request_is_silent_when_aws_cli_is_missing_or_fails() {
    let f = Fixture::new();
    let failing = fake_aws(&f, "printf 'partial\\n'; exit 3");
    for path in [failing.as_str(), "/nonexistent"] {
        let output = profile_request(path, &["aws", "sso"]);
        success(&output);
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn bash_completes_profiles_as_data_and_keeps_static_candidates() {
    if Command::new("bash").arg("--version").output().is_err() {
        eprintln!("Skipping bash: not installed");
        return;
    }
    let f = Fixture::new();
    // Names that would run or expand if the script treated them as shell code.
    let path = fake_aws(&f, r#"printf 'default\ndev admin\n$(touch injected)\n*\n'"#);
    let complete = |line: &str| {
        let script = r#"eval "$(tora completion bash)" || exit 1
read -r -a COMP_WORDS <<< "$LINE "
[[ $LINE == *" " ]] && COMP_WORDS+=("")
COMP_CWORD=$((${#COMP_WORDS[@]} - 1))
$(complete -p tora | sed 's/.*-F \([^ ]*\).*/\1/') tora "${COMP_WORDS[COMP_CWORD]}" "${COMP_WORDS[COMP_CWORD-1]}"
printf '%s\n' "${COMPREPLY[@]}""#;
        let output = Command::new("bash")
            .args(["--noprofile", "--norc", "-c", script])
            .env("LINE", line)
            .env("PATH", &path)
            .env_remove("BASH_ENV")
            .current_dir(f.0.path())
            .output()
            .unwrap();
        success(&output);
        String::from_utf8(output.stdout).unwrap()
    };
    assert_eq!(
        complete("tora aws sso "),
        "default\ndev\\ admin\n\\$\\(touch\\ injected\\)\n\\*\n"
    );
    assert_eq!(complete("tora aws exec de"), "default\ndev\\ admin\n");
    assert!(complete("tora aws sso --no-b").contains("--no-browser"));
    assert!(complete("tora aws ").contains("profiles\nsso"));
    assert!(!complete("tora aws exec default -- de").contains("default"));
    assert!(!f.path("injected").exists());
}

const SWITCH_AWS: &str = r#"printf '%s\n' "$*" >> "$HOME/calls"
case "$*" in
  *'sts get-caller-identity'*)
    [ -z "$TEST_EXPIRED" ] && exit 0
    printf '%s\n' 'Error loading SSO Token: Token for dev does not exist' >&2; exit 1;;
  *'sso login'*) printf 'login output\n'; exit "${TEST_LOGIN_STATUS:-0}";;
  *'configure list-profiles'*) printf 'default\ndev\ndev admin\n';;
  *) exit 1;;
esac"#;

/// Run `script` in a non-interactive `shell` that sourced the installed loader
/// with `set -eu`, as a strict user configuration would.
fn use_shell(f: &Fixture, shell: &str, path: &str, script: &str) -> Option<Output> {
    // Look the shell up on the same PATH it will run with.
    if Command::new(shell)
        .arg("--version")
        .env("PATH", path)
        .output()
        .is_err()
    {
        eprintln!("Skipping {shell}: not installed");
        return None;
    }
    let loader = f.path(&format!(".tora/completions/{shell}"));
    if !loader.exists() {
        success(&f.install(shell));
    }
    let output = Command::new(shell)
        .args(if shell == "bash" {
            &["--noprofile", "--norc", "-c"][..]
        } else {
            &["-f", "-c"]
        })
        .arg(format!("set -eu\n. \"$LOADER\"\n{script}"))
        .env("LOADER", loader)
        .env("HOME", f.0.path())
        .env("ZDOTDIR", f.0.path())
        .env("PATH", path)
        .env_remove("TORA_HOME")
        .env_remove("BASH_ENV")
        .env_remove("AWS_PROFILE")
        .env_remove("AWS_DEFAULT_PROFILE")
        .env_remove("AWS_REGION")
        .env_remove("AWS_DEFAULT_REGION")
        .current_dir(f.0.path())
        .output()
        .unwrap();
    Some(output)
}

fn stdout(output: &Output) -> String {
    success(output);
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn tora_use_switches_the_current_shell_only_after_successful_login() {
    for shell in ["bash", "zsh"] {
        let f = Fixture::new();
        let path = fake_aws(&f, SWITCH_AWS);
        let Some(output) = use_shell(
            &f,
            shell,
            &path,
            r#"export AWS_PROFILE=old AWS_ACCESS_KEY_ID=key AWS_REGION=eu-west-1
tora-use 'dev admin'
printf 'profile=%s key=%s region=%s\n' "$AWS_PROFILE" "${AWS_ACCESS_KEY_ID-unset}" "$AWS_DEFAULT_REGION"
export AWS_PROFILE=old AWS_ACCESS_KEY_ID=key
TEST_EXPIRED=1 tora-use dev
printf 'profile=%s key=%s\n' "$AWS_PROFILE" "${AWS_ACCESS_KEY_ID-unset}""#,
        ) else {
            continue;
        };
        assert_eq!(
            stdout(&output),
            "profile=dev admin key=unset region=eu-west-1\nprofile=dev key=unset\n",
            "{shell}"
        );
        let calls = fs::read_to_string(f.path("calls")).unwrap();
        assert!(
            calls.contains("sso login --profile dev"),
            "{shell}: {calls}"
        );
    }
}

#[test]
fn tora_use_failure_or_cancellation_leaves_the_environment_unchanged() {
    for shell in ["bash", "zsh"] {
        let f = Fixture::new();
        let path = fake_aws(&f, SWITCH_AWS);
        // Each attempt must fail without aborting the `set -e` shell.
        let Some(output) = use_shell(
            &f,
            shell,
            &path,
            r#"export AWS_PROFILE=old AWS_ACCESS_KEY_ID=key
if TEST_EXPIRED=1 TEST_LOGIN_STATUS=4 tora-use dev; then echo changed; else echo "login=$?"; fi
if tora-use <<< q; then echo changed; else echo "cancel=$?"; fi
if tora-use --select dev; then echo changed; else echo "usage=$?"; fi
if PATH=/nonexistent tora-use dev; then echo changed; else echo "missing=$?"; fi
tora-use --help | head -n 1
printf 'profile=%s key=%s\n' "$AWS_PROFILE" "$AWS_ACCESS_KEY_ID""#,
        ) else {
            continue;
        };
        assert_eq!(
            stdout(&output),
            "login=1\ncancel=1\nusage=2\nmissing=127\n\
             Usage: tora-use [--select] [--no-browser] [--use-device-code] [PROFILE]\n\
             profile=old key=key\n",
            "{shell}"
        );
    }
}

#[test]
fn tora_use_without_a_profile_selects_one_instead_of_reusing_the_environment() {
    for shell in ["bash", "zsh"] {
        let f = Fixture::new();
        let path = fake_aws(&f, SWITCH_AWS);
        let Some(output) = use_shell(
            &f,
            shell,
            &path,
            r#"export AWS_PROFILE=default
tora-use --no-browser <<< 2
printf 'profile=%s\n' "$AWS_PROFILE"
tora-use -- '-odd' >/dev/null 2>&1
printf 'profile=%s\n' "$AWS_PROFILE""#,
        ) else {
            continue;
        };
        assert_eq!(stdout(&output), "profile=dev\nprofile=-odd\n", "{shell}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("Select AWS profile"));
    }
}

#[test]
fn tora_use_completes_profiles_and_options() {
    let f = Fixture::new();
    let path = fake_aws(&f, SWITCH_AWS);
    let Some(output) = use_shell(
        &f,
        "bash",
        &path,
        r#"complete -p tora-use
reply() { COMP_WORDS=("$@"); COMP_CWORD=$(($# - 1)); _tora_use_complete; printf '[%s]' ${COMPREPLY[@]+"${COMPREPLY[@]}"}; echo; }
reply tora-use de
reply tora-use --no-browser ''
reply tora-use dev ''
reply tora-use --no
reply tora-use --select ''"#,
    ) else {
        return;
    };
    assert_eq!(
        stdout(&output),
        "complete -F _tora_use_complete tora-use\n\
         [default][dev][dev\\ admin]\n[default][dev][dev\\ admin]\n[]\n[--no-browser]\n[]\n"
    );
    if let Some(output) = use_shell(&f, "zsh", &path, "print -r -- ${_comps[tora-use]}") {
        assert_eq!(stdout(&output), "_tora_use\n");
    }
}

#[test]
fn tora_use_leaves_existing_names_and_posix_shells_alone() {
    let f = Fixture::new();
    let path = fake_aws(&f, SWITCH_AWS);
    let other = f.path("other");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("tora-use"), "#!/bin/sh\necho external\n").unwrap();
    fs::set_permissions(other.join("tora-use"), fs::Permissions::from_mode(0o755)).unwrap();
    let with_command = format!("{}:{path}", other.display());
    for shell in ["bash", "zsh"] {
        let alias = if shell == "bash" {
            "shopt -s expand_aliases; alias tora-use='echo alias'"
        } else {
            "alias tora-use='echo alias'"
        };
        // The alias is defined before the loader, as in a user's startup file.
        let Some(output) = use_shell(&f, shell, &path, "true") else {
            continue;
        };
        success(&output);
        let loader = f.path(&format!(".tora/completions/{shell}"));
        let check = Command::new(shell)
            .arg(if shell == "bash" { "--norc" } else { "-f" })
            .arg("-c")
            .arg(format!(
                "{alias}\n. \"$LOADER\"\neval 'tora-use'\n{}",
                if shell == "bash" {
                    "type -t _tora_use_complete || echo none"
                } else {
                    "(( $+functions[_tora_use] )) && echo defined || echo none"
                }
            ))
            .env("LOADER", &loader)
            .env("HOME", f.0.path())
            .env("ZDOTDIR", f.0.path())
            .env("PATH", &path)
            .env_remove("TORA_HOME")
            .env_remove("BASH_ENV")
            .output()
            .unwrap();
        assert_eq!(stdout(&check), "alias\nnone\n", "{shell}");
        let Some(output) = use_shell(&f, shell, &with_command, "tora-use") else {
            continue;
        };
        assert_eq!(stdout(&output), "external\n", "{shell}");
    }
    // sh is Bash in POSIX mode on some systems and may read the login file.
    let output = Command::new("bash")
        .args([
            "--posix",
            "-c",
            "set -eu; . \"$LOADER\"; type -t tora-use || echo none",
        ])
        .env("LOADER", f.path(".tora/completions/bash"))
        .env("HOME", f.0.path())
        .env("PATH", &path)
        .env_remove("BASH_ENV")
        .output()
        .unwrap();
    assert_eq!(stdout(&output), "none\n");
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
