use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

fn executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(name))
        .find(|path| executable(path))
}

fn report(label: &str, result: io::Result<String>, failures: &mut usize) {
    match result {
        Ok(message) => println!("[OK] {label}: {message}"),
        Err(error) => {
            *failures += 1;
            println!("[WARN] {label}: {error}");
        }
    }
}

pub fn run(profile: Option<String>) -> io::Result<()> {
    let mut failures = 0;
    println!("tora doctor ({})", env!("CARGO_PKG_VERSION"));
    report(
        "Configuration",
        crate::config::load()
            .and_then(|_| Ok(crate::config::home()?.join("config").display().to_string())),
        &mut failures,
    );
    report(
        "Installation",
        (|| {
            let installed = crate::config::home()?.join("bin/tora");
            if installed.canonicalize().ok() != Some(env::current_exe()?.canonicalize()?) {
                return Err(io::Error::other(format!(
                    "running outside {}; install using scripts/install.sh to enable self-update",
                    installed.display()
                )));
            }
            Ok(installed.display().to_string())
        })(),
        &mut failures,
    );
    report(
        "PATH",
        (|| {
            let found = on_path("tora").ok_or_else(|| {
                io::Error::other("tora not found; add $TORA_HOME/bin (default ~/.tora/bin) to PATH")
            })?;
            if found.canonicalize()? != env::current_exe()?.canonicalize()? {
                return Err(io::Error::other(format!(
                    "{} shadows this binary; reorder PATH",
                    found.display()
                )));
            }
            Ok(found.display().to_string())
        })(),
        &mut failures,
    );
    let missing: Vec<_> = [
        "bash", "curl", "tar", "awk", "mktemp", "wc", "tr", "cat", "chmod", "mv", "mkdir", "rmdir",
        "rm", "uname",
    ]
    .into_iter()
    .filter(|name| on_path(name).is_none())
    .collect();
    report(
        "Update tools",
        if missing.is_empty() && (on_path("sha256sum").is_some() || on_path("shasum").is_some()) {
            Ok("installer prerequisites found".into())
        } else {
            Err(io::Error::other(format!(
                "install missing tools: {}; also requires sha256sum or shasum",
                missing.join(", ")
            )))
        },
        &mut failures,
    );
    report("Completion", crate::completion::diagnose(), &mut failures);
    let cli = Command::new("aws")
        .arg("--version")
        .output()
        .and_then(|output| {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if output.status.success() && text.starts_with("aws-cli/2.") {
                Ok(text.trim().to_owned())
            } else {
                Err(io::Error::other(
                    "install AWS CLI v2 and ensure it is first in PATH",
                ))
            }
        })
        .map_err(|e| io::Error::other(format!("{e}; install AWS CLI v2")));
    let has_cli = cli.is_ok();
    report("AWS CLI", cli, &mut failures);
    if has_cli {
        report(
            "AWS authentication / SSO",
            crate::aws::identity(profile),
            &mut failures,
        );
    } else {
        println!("[SKIP] AWS authentication / SSO: AWS CLI v2 is required");
    }
    if failures == 0 {
        println!("All checks passed.");
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{failures} check(s) need attention; see the suggestions above"
        )))
    }
}
