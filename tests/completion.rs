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
        ("zsh", "[[ ${_comps[tora]} == _tora ]] && functions _tora"),
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
