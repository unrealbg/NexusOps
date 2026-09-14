//! OS-backed encrypted credentials. No plaintext persistence or fallback backend.
mod vault;
use nexus_model::{AppError, AuthenticationMethod, ErrorCode, HostId};
use serde::Deserialize;
use ts_rs::TS;
pub use vault::{EncryptedSecretStore, KeyProvider, PlatformKeyProvider};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// One-shot IPC input. Intentionally neither Debug nor Serialize.
#[derive(Deserialize, TS, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
pub struct CredentialInput {
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
}
impl CredentialInput {
    pub fn into_credential(mut self, method: AuthenticationMethod) -> Result<Credential, AppError> {
        let credential = Credential {
            password: self.password.take().map(Zeroizing::new),
            private_key: self.private_key.take().map(Zeroizing::new),
            passphrase: self.passphrase.take().map(Zeroizing::new),
        };
        credential.validate(method)?;
        Ok(credential)
    }
}

/// Short-lived secret material; do not add Debug or public serialization.
pub struct Credential {
    pub password: Option<Zeroizing<String>>,
    pub private_key: Option<Zeroizing<String>>,
    pub passphrase: Option<Zeroizing<String>>,
}
impl Credential {
    pub fn validate(&self, method: AuthenticationMethod) -> Result<(), AppError> {
        let nonempty = |v: &Option<Zeroizing<String>>| v.as_ref().is_some_and(|s| !s.is_empty());
        let valid = match method {
            AuthenticationMethod::Password => {
                nonempty(&self.password) && self.private_key.is_none() && self.passphrase.is_none()
            }
            AuthenticationMethod::PrivateKey => {
                nonempty(&self.private_key) && self.password.is_none()
            }
        };
        if !valid {
            return Err(AppError::new(
                ErrorCode::Validation,
                "Provide credentials matching the selected authentication method.",
            ));
        }
        if [&self.password, &self.private_key, &self.passphrase]
            .iter()
            .any(|s| s.as_ref().is_some_and(|s| s.len() > 65_536))
        {
            return Err(AppError::new(
                ErrorCode::Validation,
                "Credential exceeds the 64 KiB size limit.",
            ));
        }
        Ok(())
    }
}

/// Immutable credential revisions make metadata/secret updates crash-safe.
/// Store a new ID, commit its metadata reference, then remove the old revision.
pub trait SecretStore: Send + Sync {
    fn put(&self, id: HostId, credential: &Credential) -> Result<(), AppError>;
    fn get(&self, id: HostId) -> Result<Credential, AppError>;
    fn delete(&self, id: HostId) -> Result<(), AppError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credential_authentication_is_not_ambiguous() {
        let bad = CredentialInput {
            password: Some("test".into()),
            private_key: Some("test".into()),
            passphrase: None,
        };
        assert!(bad.into_credential(AuthenticationMethod::Password).is_err());
        let bad = CredentialInput {
            password: None,
            private_key: None,
            passphrase: None,
        };
        assert!(
            bad.into_credential(AuthenticationMethod::PrivateKey)
                .is_err()
        );
    }
}
