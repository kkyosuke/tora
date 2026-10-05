use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const LIMIT: usize = 20;
const MAX_BYTES: u64 = 64 * 1024;

fn path() -> Option<PathBuf> {
    std::env::var_os("TORA_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|s| !s.is_empty())
                .map(|home| PathBuf::from(home).join(".tora"))
        })
        .map(|home| home.join("aws-profile-history"))
}

fn read(path: &Path) -> Vec<String> {
    let mut content = String::new();
    let result =
        fs::File::open(path).and_then(|file| file.take(MAX_BYTES + 1).read_to_string(&mut content));
    if result.is_err() || content.len() as u64 > MAX_BYTES {
        return Vec::new();
    }
    let mut profiles = Vec::new();
    for profile in content.lines() {
        if super::validate_profile(profile).is_ok() && !profiles.iter().any(|p| p == profile) {
            profiles.push(profile.to_owned());
            if profiles.len() == LIMIT {
                break;
            }
        }
    }
    profiles
}

pub(super) fn sort(profiles: &mut [String]) {
    let recent = path().map(|path| read(&path)).unwrap_or_default();
    // Stable sort preserves AWS CLI order for profiles that have not been used.
    profiles.sort_by_key(|profile| recent.iter().position(|p| p == profile).unwrap_or(LIMIT));
}

fn save(path: &Path, profile: &str) -> io::Result<()> {
    let mut recent = read(path);
    recent.retain(|p| p != profile);
    recent.insert(0, profile.to_owned());
    recent.truncate(LIMIT);
    let content = format!("{}\n", recent.join("\n"));
    if content.len() as u64 > MAX_BYTES {
        return Err(io::Error::other("profile history exceeds size limit"));
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

pub(super) fn remember(profile: &str) {
    if let Some(path) = path() {
        if let Err(error) = save(&path, profile) {
            eprintln!("tora: could not save AWS profile history: {error}");
        }
    }
}
