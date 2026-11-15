use crate::{
    cli::{ConfigCommand, ConfigKey},
    paths,
};
use clap::ValueEnum;
use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
};

#[derive(Default)]
pub struct Config {
    pub profile: Option<String>,
    pub region: Option<String>,
}

impl ConfigKey {
    fn name(self) -> String {
        self.to_possible_value()
            .expect("config keys are visible CLI values")
            .get_name()
            .to_owned()
    }
    fn field(self, config: &mut Config) -> &mut Option<String> {
        match self {
            Self::Profile => &mut config.profile,
            Self::Region => &mut config.region,
        }
    }
}

fn validate(key: ConfigKey, value: &str) -> io::Result<()> {
    if value.trim().is_empty() || value.chars().any(char::is_control) || value.trim() != value {
        return Err(io::Error::other(
            "value must be non-empty, with no control characters or surrounding whitespace",
        ));
    }
    if matches!(key, ConfigKey::Region)
        && !value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    {
        return Err(io::Error::other(
            "region must contain only lowercase letters, digits and hyphens",
        ));
    }
    Ok(())
}

fn parse(text: &str) -> io::Result<Config> {
    let mut config = Config::default();
    for line in text
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
    {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| io::Error::other("invalid config: expected key=value"))?;
        let key = ConfigKey::from_str(key, false)
            .map_err(|_| io::Error::other("invalid config: unknown key"))?;
        validate(key, value)?;
        if key.field(&mut config).replace(value.to_owned()).is_some() {
            return Err(io::Error::other("invalid config: duplicate key"));
        }
    }
    Ok(config)
}

pub fn load() -> io::Result<Config> {
    let path = paths::config()?;
    match fs::read_to_string(&path) {
        Ok(text) => parse(&text).map_err(|e| io::Error::other(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound && !path.is_symlink() => {
            Ok(Config::default())
        }
        Err(e) => Err(e),
    }
}

fn contents(config: &Config) -> String {
    let mut text = String::new();
    for (key, value) in [
        (ConfigKey::Profile, &config.profile),
        (ConfigKey::Region, &config.region),
    ] {
        if let Some(value) = value {
            text.push_str(&format!("{}={value}\n", key.name()));
        }
    }
    text
}

struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}

pub fn run(command: Option<ConfigCommand>) -> io::Result<()> {
    let path = paths::config()?;
    match command.unwrap_or(ConfigCommand::List) {
        ConfigCommand::List => {
            println!("Config: {}", path.display());
            let text = contents(&load()?);
            print!(
                "{}",
                if text.is_empty() {
                    "No defaults configured.\n"
                } else {
                    &text
                }
            );
        }
        ConfigCommand::Get { key } => {
            let mut config = load()?;
            let value = key
                .field(&mut config)
                .as_ref()
                .ok_or_else(|| io::Error::other(format!("{} is not set", key.name())))?;
            println!("{value}");
        }
        ConfigCommand::Set { key, value } => {
            validate(key, &value)?;
            save(key, Some(value))?;
        }
        ConfigCommand::Unset { key } => save(key, None)?,
    }
    Ok(())
}

fn save(key: ConfigKey, value: Option<String>) -> io::Result<()> {
    let path = paths::config()?;
    let parent = path.parent().expect("config has parent");
    fs::create_dir_all(parent)?;
    let lock_path = parent.join("config.lock");
    fs::create_dir(&lock_path).map_err(|e| io::Error::other(format!("cannot acquire {}: {e}; if no config writer is running, remove the empty lock directory", lock_path.display())))?;
    let _lock = Lock(lock_path);
    if path.is_symlink() {
        return Err(io::Error::other("config must not be a symlink"));
    }
    let mut config = load()?;
    *key.field(&mut config) = value;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(contents(&config).as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(&path).map_err(|e| e.error)?;
    println!("Saved {}", path.display());
    Ok(())
}
