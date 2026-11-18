use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const LIMIT: usize = 20;
const MAX_BYTES: u64 = 64 * 1024;

/// A small most-recently-used list stored as one entry per line under `TORA_HOME`.
pub(super) struct Store {
    file: &'static str,
    label: &'static str,
    valid: fn(&str) -> bool,
}

/// Profiles shared by `aws sso`, `aws exec` and `aws ssm`.
pub(super) const PROFILES: Store = Store {
    file: "aws-profile-history",
    label: "AWS profile",
    valid: |profile| super::validate_profile(profile).is_ok(),
};

/// Session Manager targets, stored as `<profile>\t<instance-id>`.
pub(super) const INSTANCES: Store = Store {
    file: "aws-ssm-history",
    label: "Session Manager",
    valid: |entry| {
        entry.split_once('\t').is_some_and(|(profile, target)| {
            super::validate_profile(profile).is_ok() && super::ssm::valid_target(target)
        })
    },
};

impl Store {
    fn path(&self) -> Option<PathBuf> {
        std::env::var_os("TORA_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|s| !s.is_empty())
                    .map(|home| PathBuf::from(home).join(".tora"))
            })
            .map(|home| home.join(self.file))
    }

    fn read(&self, path: &Path) -> Vec<String> {
        let mut content = String::new();
        let result = fs::File::open(path)
            .and_then(|file| file.take(MAX_BYTES + 1).read_to_string(&mut content));
        if result.is_err() || content.len() as u64 > MAX_BYTES {
            return Vec::new();
        }
        let mut entries = Vec::new();
        for entry in content.lines() {
            if (self.valid)(entry) && !entries.iter().any(|e| e == entry) {
                entries.push(entry.to_owned());
                if entries.len() == LIMIT {
                    break;
                }
            }
        }
        entries
    }

    /// Most recently used entries first.
    pub(super) fn recent(&self) -> Vec<String> {
        self.path().map(|path| self.read(&path)).unwrap_or_default()
    }

    fn save(&self, path: &Path, entry: &str) -> io::Result<()> {
        let mut recent = self.read(path);
        recent.retain(|e| e != entry);
        recent.insert(0, entry.to_owned());
        recent.truncate(LIMIT);
        let content = format!("{}\n", recent.join("\n"));
        if content.len() as u64 > MAX_BYTES {
            return Err(io::Error::other("history exceeds size limit"));
        }
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("invalid history path"))?;
        fs::create_dir_all(parent)?;
        // Private, same-directory temporary file; readers only see complete snapshots.
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(content.as_bytes())?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    }

    pub(super) fn remember(&self, entry: &str) {
        if !(self.valid)(entry) {
            return;
        }
        if let Some(path) = self.path() {
            if let Err(error) = self.save(&path, entry) {
                eprintln!("tora: could not save {} history: {error}", self.label);
            }
        }
    }
}

/// Moves recently used items to the front, most recent first. Returns the number of
/// items that appear in `recent`.
pub(super) fn sort_by_recent<T>(
    items: &mut [T],
    recent: &[String],
    key: impl Fn(&T) -> &str,
) -> usize {
    // Stable sort preserves the original order for items that have not been used.
    items.sort_by_key(|item| recent.iter().position(|r| r == key(item)).unwrap_or(LIMIT));
    items
        .iter()
        .take_while(|item| recent.iter().any(|r| r == key(item)))
        .count()
}

pub(super) fn sort(profiles: &mut [String]) -> usize {
    sort_by_recent(profiles, &PROFILES.recent(), |p| p)
}

pub(super) fn remember(profile: &str) {
    PROFILES.remember(profile);
}
