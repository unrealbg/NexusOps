use crate::{AppError, ErrorCode};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct HostId(pub Uuid);
impl HostId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for HostId {
    fn default() -> Self {
        Self::new()
    }
}
impl std::fmt::Display for HostId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::str::FromStr for HostId {
    type Err = AppError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s)
            .map(Self)
            .map_err(|_| AppError::new(ErrorCode::Validation, "Invalid host identifier."))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AuthenticationMethod {
    Password,
    PrivateKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HostConnectionConfig {
    pub hostname: String,
    pub port: u16,
    pub username: String,
    pub authentication: AuthenticationMethod,
}
impl HostConnectionConfig {
    /// Normalize DNS spelling; never interpret user text as a shell fragment or URL.
    pub fn normalize(&mut self) {
        self.hostname = self
            .hostname
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        self.username = self.username.trim().to_owned();
    }
    pub fn validate(&self) -> Result<(), AppError> {
        let is_ip = self.hostname.parse::<std::net::IpAddr>().is_ok();
        let is_dns = !self.hostname.is_empty()
            && self.hostname.len() <= 253
            && self.hostname.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            });
        if !is_ip && !is_dns {
            return Err(AppError::new(
                ErrorCode::Validation,
                "Enter a hostname or IP address without a URL, path or spaces.",
            ));
        }
        if self.port == 0 {
            return Err(AppError::new(
                ErrorCode::Validation,
                "SSH port must be between 1 and 65535.",
            ));
        }
        if self.username.is_empty()
            || self.username.len() > 128
            || self
                .username
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(AppError::new(
                ErrorCode::Validation,
                "Enter a username without whitespace or control characters.",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub id: HostId,
    pub display_name: String,
    pub connection: HostConnectionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HostInput {
    pub id: Option<HostId>,
    pub display_name: String,
    pub connection: HostConnectionConfig,
}
impl HostInput {
    pub fn into_host(mut self) -> Result<Host, AppError> {
        self.display_name = self.display_name.trim().to_owned();
        if self.display_name.is_empty()
            || self.display_name.len() > 120
            || self.display_name.chars().any(char::is_control)
        {
            return Err(AppError::new(
                ErrorCode::Validation,
                "Display name must contain 1–120 characters without control characters.",
            ));
        }
        self.connection.normalize();
        self.connection.validate()?;
        Ok(Host {
            id: self.id.unwrap_or_default(),
            display_name: self.display_name,
            connection: self.connection,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(hostname: &str) -> HostInput {
        HostInput {
            id: None,
            display_name: " Server ".into(),
            connection: HostConnectionConfig {
                hostname: hostname.into(),
                port: 22,
                username: " admin ".into(),
                authentication: AuthenticationMethod::Password,
            },
        }
    }
    #[test]
    fn validates_and_normalizes_endpoints() {
        assert_eq!(
            input("EXAMPLE.COM.")
                .into_host()
                .expect("host")
                .connection
                .hostname,
            "example.com"
        );
        for address in ["127.0.0.1", "::1", "my-server.local"] {
            assert!(input(address).into_host().is_ok());
        }
        for address in [
            "",
            "user@server",
            "ssh://server",
            "host;id",
            "-bad",
            "a..b",
            "host/path",
            "ser\nver",
        ] {
            assert!(input(address).into_host().is_err(), "{address}");
        }
    }
    #[test]
    fn rejects_invalid_fields_and_ids() {
        let mut h = input("host");
        h.connection.port = 0;
        assert!(h.into_host().is_err());
        let mut h = input("host");
        h.display_name = "\n".into();
        assert!(h.into_host().is_err());
        let mut h = input("host");
        h.connection.username = "a b".into();
        assert!(h.into_host().is_err());
        assert!("not-id".parse::<HostId>().is_err());
    }
}
