#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::symlink,
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
    use std::os::unix::fs::PermissionsExt;
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
