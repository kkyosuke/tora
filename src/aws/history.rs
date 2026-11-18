use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 64 * 1024;

/// A small most-recently-used list stored as one entry per line under `TORA_HOME`.
pub(super) struct Store {
    file: &'static str,
    label: &'static str,
    limit: usize,
    valid: fn(&str) -> bool,
}

/// Profiles shared by `aws sso`, `aws exec`, `aws status` and `aws ssm`.
pub(super) const PROFILES: Store = Store {
    file: "aws-profile-history",
    label: "AWS profile",
    limit: 20,
    valid: |profile| super::validate_profile(profile).is_ok(),
};

/// Session Manager targets per profile, stored as `<profile>\t<target>`.
/// [`PER_PROFILE`] entries are kept for each profile.
pub(super) const INSTANCES: Store = Store {
    file: "aws-ssm-history",
    label: "Session Manager",
    // No global count limit: entries are capped per profile, and the oldest entries are
    // dropped only if the file would exceed MAX_BYTES.
    limit: usize::MAX,
    valid: |entry| {
        split_profile(entry).is_some_and(|(profile, target)| {
            super::validate_profile(profile).is_ok() && super::ssm::valid_target(target)
        })
    },
};

/// Port forwarding destinations per profile, stored as `<profile>\t<target>\t<host>:<port>`.
/// [`PER_PROFILE`] entries are kept for each profile.
pub(super) const TUNNELS: Store = Store {
    file: "aws-tunnel-history",
    label: "port forwarding",
    limit: usize::MAX,
    valid: |entry| {
        split_profile(entry).is_some_and(|(profile, value)| {
            super::validate_profile(profile).is_ok() && super::tunnel::parse_entry(value).is_some()
        })
    },
};
pub(super) const PER_PROFILE: usize = 20;

/// Splits a per-profile history entry into `(profile, value)`.
fn split_profile(entry: &str) -> Option<(&str, &str)> {
    entry.split_once('\t')
}

impl Store {
    fn path(&self) -> Option<PathBuf> {
        crate::paths::home().ok().map(|home| home.join(self.file))
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
                if entries.len() == self.limit {
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

    fn save(
        &self,
        path: &Path,
        entry: &str,
        keep: impl Fn(&[String], &str) -> bool,
    ) -> io::Result<()> {
        let mut recent = self.read(path);
        recent.retain(|e| e != entry);
        recent.insert(0, entry.to_owned());
        let mut kept: Vec<String> = Vec::new();
        for entry in recent {
            if keep(&kept, &entry) {
                kept.push(entry);
            }
        }
        kept.truncate(self.limit);
        let mut content = format!("{}\n", kept.join("\n"));
        // Drop the oldest entries rather than refusing to record the newest one.
        while content.len() as u64 > MAX_BYTES && kept.len() > 1 {
            kept.pop();
            content = format!("{}\n", kept.join("\n"));
        }
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

    fn remember_with(&self, entry: &str, keep: impl Fn(&[String], &str) -> bool) {
        if !(self.valid)(entry) {
            return;
        }
        if let Some(path) = self.path() {
            if let Err(error) = self.save(&path, entry, keep) {
                eprintln!("tora: could not save {} history: {error}", self.label);
            }
        }
    }

    pub(super) fn remember(&self, entry: &str) {
        self.remember_with(entry, |_, _| true);
    }

    /// Values saved for `profile`, most recent first.
    pub(super) fn for_profile(&self, profile: &str) -> Vec<String> {
        self.recent()
            .into_iter()
            .filter_map(|entry| {
                let (p, value) = split_profile(&entry)?;
                (p == profile).then(|| value.to_owned())
            })
            .collect()
    }

    /// Saves a value for `profile` and keeps at most [`PER_PROFILE`] entries for each
    /// profile, so frequent use of one profile does not push out another profile's history.
    pub(super) fn remember_for(&self, profile: &str, value: &str) {
        let key = |entry: &str| split_profile(entry).map(|(p, _)| p.to_owned());
        self.remember_with(&format!("{profile}\t{value}"), |kept, entry| {
            let profile = key(entry);
            kept.iter().filter(|e| key(e) == profile).count() < PER_PROFILE
        });
    }
}

/// Moves items found in `recent` to the front, most recent first. The stable sort keeps
/// the original order for items that have not been used.
pub(super) fn sort_by_recent<T>(items: &mut [T], recent: &[String], key: impl Fn(&T) -> &str) {
    items.sort_by_cached_key(|item| {
        recent
            .iter()
            .position(|r| r == key(item))
            .unwrap_or(usize::MAX)
    });
}

/// `(last used)` only for the actual most recent entry, even when that entry is no longer
/// listed; `(recent)` for any older entry.
pub(super) fn marker(item: &str, recent: &[String]) -> &'static str {
    match recent.iter().position(|r| r == item) {
        Some(0) => "  (last used)",
        Some(_) => "  (recent)",
        None => "",
    }
}

pub(super) fn recent_profiles() -> Vec<String> {
    PROFILES.recent()
}

pub(super) fn remember(profile: &str) {
    PROFILES.remember(profile);
}
