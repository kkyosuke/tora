//! Shared locations and installed-binary identity for every command.
use std::{
    env, io,
    path::{Path, PathBuf},
};

fn resolve_home(tora_home: Option<PathBuf>, user_home: Option<PathBuf>) -> io::Result<PathBuf> {
    tora_home
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| {
            user_home
                .filter(|p| !p.as_os_str().is_empty())
                .map(|p| p.join(".tora"))
        })
        .ok_or_else(|| io::Error::other("HOME or TORA_HOME is required"))
}

pub fn home() -> io::Result<PathBuf> {
    resolve_home(
        env::var_os("TORA_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
    )
}

pub fn config() -> io::Result<PathBuf> {
    Ok(home()?.join("config"))
}

pub fn installed_binary() -> io::Result<PathBuf> {
    Ok(home()?.join("bin/tora"))
}

pub fn is_current_binary(path: &Path) -> io::Result<bool> {
    Ok(path.canonicalize().ok() == Some(env::current_exe()?.canonicalize()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_prefers_nonempty_override_and_requires_a_fallback() {
        assert_eq!(
            resolve_home(Some("custom".into()), Some("user".into())).unwrap(),
            Path::new("custom")
        );
        for override_value in [None, Some(PathBuf::new())] {
            assert_eq!(
                resolve_home(override_value, Some("user".into())).unwrap(),
                Path::new("user/.tora")
            );
        }
        assert!(resolve_home(None, None).is_err());
        assert!(resolve_home(Some(PathBuf::new()), Some(PathBuf::new())).is_err());
    }
}
