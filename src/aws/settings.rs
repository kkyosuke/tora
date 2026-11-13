use crate::{config, shell};
use std::{env, io, process::Command};

// Static credentials override AWS_PROFILE. Remove them only for an explicitly
// selected profile, or from the requested shell export; never from the parent.
const CREDENTIAL_ENV: &[&str] = &[
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_SECURITY_TOKEN",
    "AWS_DEFAULT_PROFILE",
];

/// Resolve saved defaults and environment once for one AWS operation.
#[derive(Default)]
pub(super) struct Settings {
    pub profile: Option<String>,
    pub region: Option<String>,
}

fn env_value(names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| env::var(name).ok().filter(|v| !v.is_empty()))
}

fn aws() -> Command {
    let mut command = Command::new("aws");
    command
        .env("AWS_PAGER", "")
        .env("AWS_CLI_AUTO_PROMPT", "off");
    command
}

impl Settings {
    pub fn load() -> io::Result<Self> {
        let config = config::load()?;
        Ok(Self {
            profile: env_value(&["AWS_PROFILE", "AWS_DEFAULT_PROFILE"]).or(config.profile),
            region: env_value(&["AWS_REGION", "AWS_DEFAULT_REGION"]).or(config.region),
        })
    }

    fn apply_region(&self, command: &mut Command) {
        if let Some(region) = &self.region {
            command
                .env("AWS_REGION", region)
                .env("AWS_DEFAULT_REGION", region);
        }
    }

    pub fn command(&self) -> Command {
        let mut command = aws();
        self.apply_region(&mut command);
        command
    }

    pub fn profile_command(&self, profile: &str) -> Command {
        let mut command = aws();
        self.configure_profile(&mut command, profile);
        command
    }

    pub fn configure_profile(&self, command: &mut Command, profile: &str) {
        command.env("AWS_PROFILE", profile);
        for name in CREDENTIAL_ENV {
            command.env_remove(name);
        }
        self.apply_region(command);
    }

    pub fn export_statement(&self, profile: &str) -> String {
        let mut statement = format!(
            "unset {};\nexport AWS_PROFILE={}",
            CREDENTIAL_ENV.join(" "),
            shell::quote(profile)
        );
        if let Some(region) = &self.region {
            let region = shell::quote(region);
            statement.push_str(&format!(
                ";\nexport AWS_REGION={region} AWS_DEFAULT_REGION={region}"
            ));
        }
        statement
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_quote_profile_and_region_as_literal_values() {
        let settings = Settings {
            profile: None,
            region: Some("x'y$(echo x)".into()),
        };
        let text = settings.export_statement("a'b$(echo x)");
        assert!(text.contains("export AWS_PROFILE='a'\\''b$(echo x)'"));
        assert!(text.contains(
            "export AWS_REGION='x'\\''y$(echo x)' AWS_DEFAULT_REGION='x'\\''y$(echo x)'"
        ));
    }
}
