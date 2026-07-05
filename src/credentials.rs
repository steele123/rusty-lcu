use std::{env, path::PathBuf};

use sysinfo::System;

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub port: u16,
    pub password: String,
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialsSource {
    Auto,
    Process,
    DefaultLockfile,
    Lockfile(PathBuf),
    LockfileContent(String),
    Manual(Credentials),
}

impl Credentials {
    pub fn new(port: u16, password: impl Into<String>) -> Self {
        Self {
            port,
            password: password.into(),
            protocol: "https".to_string(),
        }
    }

    pub fn base_url(&self) -> String {
        format!("{}://127.0.0.1:{}", self.protocol, self.port)
    }

    pub fn websocket_url(&self) -> String {
        format!("wss://127.0.0.1:{}", self.port)
    }

    pub async fn discover(source: CredentialsSource) -> Result<Self> {
        match source {
            CredentialsSource::Auto => Self::from_process()
                .or_else(|_| Self::from_default_lockfile_blocking())
                .or(Err(Error::CredentialsNotFound)),
            CredentialsSource::Process => Self::from_process(),
            CredentialsSource::DefaultLockfile => Self::from_default_lockfile().await,
            CredentialsSource::Lockfile(path) => Self::from_lockfile(path).await,
            CredentialsSource::LockfileContent(content) => Self::from_lockfile_content(&content),
            CredentialsSource::Manual(credentials) => Ok(credentials),
        }
    }

    pub async fn from_default_lockfile() -> Result<Self> {
        let path = default_lockfile_path().ok_or(Error::CredentialsNotFound)?;
        Self::from_lockfile(path).await
    }

    pub async fn from_lockfile(path: impl Into<PathBuf>) -> Result<Self> {
        let content = tokio::fs::read_to_string(path.into()).await?;
        Self::from_lockfile_content(&content)
    }

    pub fn from_process() -> Result<Self> {
        let system = System::new_all();

        for process in system.processes().values() {
            let name = process.name().to_string_lossy();
            let command_line = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");

            if !is_league_client_process(&name, &command_line) {
                continue;
            }

            if let Some(credentials) = parse_credentials_from_command_line(&command_line) {
                return Ok(credentials);
            }
        }

        Err(Error::CredentialsNotFound)
    }

    pub fn from_lockfile_content(content: &str) -> Result<Self> {
        let parts = content.trim().split(':').collect::<Vec<_>>();
        if parts.len() < 5 {
            return Err(Error::InvalidLockfile);
        }

        let port = parts[2].parse().map_err(|_| Error::InvalidLockfile)?;
        Ok(Self {
            port,
            password: parts[3].to_string(),
            protocol: parts[4].to_string(),
        })
    }

    fn from_default_lockfile_blocking() -> Result<Self> {
        let path = default_lockfile_path().ok_or(Error::CredentialsNotFound)?;
        let content = std::fs::read_to_string(path)?;
        Self::from_lockfile_content(&content)
    }
}

fn is_league_client_process(name: &str, command_line: &str) -> bool {
    name.eq_ignore_ascii_case("LeagueClientUx.exe")
        || name.eq_ignore_ascii_case("LeagueClientUx")
        || command_line.contains("LeagueClientUx")
}

fn parse_credentials_from_command_line(command_line: &str) -> Option<Credentials> {
    let port = argument_value(command_line, "--app-port")?.parse().ok()?;
    let password = argument_value(command_line, "--remoting-auth-token")?;

    Some(Credentials {
        port,
        password,
        protocol: "https".to_string(),
    })
}

fn argument_value(command_line: &str, name: &str) -> Option<String> {
    let parts = command_line
        .split_whitespace()
        .map(|part| part.trim_matches('"'))
        .collect::<Vec<_>>();

    for (index, part) in parts.iter().enumerate() {
        if let Some(value) = part.strip_prefix(&format!("{name}=")) {
            return Some(value.trim_matches('"').to_string());
        }

        if *part == name {
            return parts
                .get(index + 1)
                .map(|value| value.trim_matches('"').to_string());
        }
    }

    None
}

fn default_lockfile_path() -> Option<PathBuf> {
    if let Ok(path) = env::var("RUSTY_LCU_LOCKFILE") {
        return Some(PathBuf::from(path));
    }

    #[cfg(windows)]
    {
        let drive = env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
        Some(PathBuf::from(format!(
            r"{drive}\Riot Games\League of Legends\lockfile"
        )))
    }

    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lockfile_content() {
        let credentials =
            Credentials::from_lockfile_content("LeagueClient:1234:51111:secret:https").unwrap();

        assert_eq!(credentials.port, 51111);
        assert_eq!(credentials.password, "secret");
        assert_eq!(credentials.protocol, "https");
    }

    #[test]
    fn parses_process_arguments_with_equals() {
        let credentials = parse_credentials_from_command_line(
            r#"LeagueClientUx.exe --app-port=51111 --remoting-auth-token=secret"#,
        )
        .unwrap();

        assert_eq!(credentials, Credentials::new(51111, "secret"));
    }

    #[test]
    fn parses_process_arguments_with_spaces() {
        let credentials = parse_credentials_from_command_line(
            r#"LeagueClientUx.exe --app-port 51111 --remoting-auth-token secret"#,
        )
        .unwrap();

        assert_eq!(credentials, Credentials::new(51111, "secret"));
    }
}
