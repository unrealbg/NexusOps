use crate::{Credential, SecretStore};
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use nexus_model::{AppError, ErrorCode, HostId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;

fn failure() -> AppError {
    AppError::new(
        ErrorCode::SecureStorage,
        "Secure credential storage is unavailable or locked. Unlock your OS keychain and try again.",
    )
}

/// Supplies a 256-bit encryption key. Production uses the OS secure store only.
pub trait KeyProvider: Send + Sync {
    /// Load an existing key. Missing keys must fail without changing storage.
    fn key(&self) -> Result<Zeroizing<Vec<u8>>, AppError>;

    /// Initialize a new empty vault. The vault calls this only before its durable
    /// initialization marker exists; test providers can reuse their fixed key.
    fn initialize(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.key()
    }
}

pub struct PlatformKeyProvider {
    account: String,
    gate: Mutex<()>,
}
impl PlatformKeyProvider {
    pub fn new(profile_id: &str) -> Self {
        Self {
            account: format!("vault-{profile_id}"),
            gate: Mutex::new(()),
        }
    }

    fn load(&self, allow_creation: bool) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let _guard = self.gate.lock().map_err(|_| failure())?;
        let entry =
            keyring::Entry::new("org.nexusops.desktop", &self.account).map_err(|_| failure())?;
        let key = match entry.get_secret() {
            Ok(key) => Zeroizing::new(key),
            Err(keyring::Error::NoEntry) if allow_creation => {
                let mut key = Zeroizing::new(vec![0; 32]);
                getrandom::fill(&mut key).map_err(|_| failure())?;
                entry.set_secret(&key).map_err(|_| failure())?;
                key
            }
            Err(_) => return Err(failure()),
        };
        if key.len() != 32 {
            return Err(failure());
        }
        Ok(key)
    }
}
impl KeyProvider for PlatformKeyProvider {
    fn key(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.load(false)
    }

    fn initialize(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.load(true)
    }
}

// JSON can expand each source byte into a six-byte Unicode escape. Keep the
// read bound consistent with all accepted 64 KiB credential fields plus framing.
const MAX_CIPHERTEXT_BYTES: i64 = 3 * 65_536 * 6 + 256;

/// AES-256-GCM ciphertext in SQLite, with the wrapping key in Credential Manager,
/// Keychain or Secret Service. This supports private keys larger than OS blob limits.
pub struct EncryptedSecretStore {
    db: Mutex<Connection>,
    keys: Arc<dyn KeyProvider>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecretRef<'a> {
    password: Option<&'a str>,
    private_key: Option<&'a str>,
    passphrase: Option<&'a str>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecretValue {
    password: Option<Zeroizing<String>>,
    private_key: Option<Zeroizing<String>>,
    passphrase: Option<Zeroizing<String>>,
}
impl EncryptedSecretStore {
    pub fn open(path: impl AsRef<Path>, keys: Arc<dyn KeyProvider>) -> Result<Self, AppError> {
        let db = Connection::open(path).map_err(|_| failure())?;
        db.execute_batch("PRAGMA journal_mode=WAL; BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS secrets(id TEXT PRIMARY KEY, nonce BLOB NOT NULL, ciphertext BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS vault_state(id INTEGER PRIMARY KEY CHECK(id=1), initialized INTEGER NOT NULL CHECK(initialized=1));
            INSERT OR IGNORE INTO vault_state(id, initialized) SELECT 1,1 WHERE EXISTS(SELECT 1 FROM secrets);
            COMMIT;").map_err(|_|failure())?;
        Ok(Self {
            db: Mutex::new(db),
            keys,
        })
    }
}
impl SecretStore for EncryptedSecretStore {
    fn put(&self, id: HostId, credential: &Credential) -> Result<(), AppError> {
        credential.validate(if credential.private_key.is_some() {
            nexus_model::AuthenticationMethod::PrivateKey
        } else {
            nexus_model::AuthenticationMethod::Password
        })?;
        let mut db = self.db.lock().map_err(|_| failure())?;
        let transaction = db.transaction().map_err(|_| failure())?;
        let initialized: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM vault_state WHERE id=1)",
                [],
                |row| row.get(0),
            )
            .map_err(|_| failure())?;
        let key = if initialized {
            self.keys.key()?
        } else {
            self.keys.initialize()?
        };
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| failure())?;
        let mut nonce_bytes = [0; 12];
        getrandom::fill(&mut nonce_bytes).map_err(|_| failure())?;
        let nonce = Nonce::from(nonce_bytes);
        let value = SecretRef {
            password: credential.password.as_ref().map(|s| s.as_str()),
            private_key: credential.private_key.as_ref().map(|s| s.as_str()),
            passphrase: credential.passphrase.as_ref().map(|s| s.as_str()),
        };
        let plaintext = Zeroizing::new(serde_json::to_vec(&value).map_err(|_| failure())?);
        let aad = format!("nexusops:v1:{id}");
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: &plaintext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| failure())?;
        transaction
            .execute(
                "INSERT INTO secrets(id,nonce,ciphertext) VALUES (?1,?2,?3)",
                params![id.to_string(), nonce.as_slice(), ciphertext],
            )
            .map_err(|_| failure())?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO vault_state(id, initialized) VALUES (1,1)",
                [],
            )
            .map_err(|_| failure())?;
        transaction.commit().map_err(|_| failure())?;
        Ok(())
    }
    fn get(&self, id: HostId) -> Result<Credential, AppError> {
        let key = self.keys.key()?;
        let row: Option<(Vec<u8>, Vec<u8>)> = self
            .db
            .lock()
            .map_err(|_| failure())?
            .query_row(
                "SELECT nonce,ciphertext FROM secrets WHERE id=?1 AND length(nonce)=12 AND length(ciphertext)<=?2",
                params![id.to_string(), MAX_CIPHERTEXT_BYTES],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| failure())?;
        let (nonce, ciphertext) = row.ok_or_else(failure)?;
        let nonce_bytes: [u8; 12] = nonce.try_into().map_err(|_| failure())?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| failure())?;
        let aad = format!("nexusops:v1:{id}");
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    &Nonce::from(nonce_bytes),
                    Payload {
                        msg: &ciphertext,
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| failure())?,
        );
        let value: SecretValue = serde_json::from_slice(&plaintext).map_err(|_| failure())?;
        Ok(Credential {
            password: value.password,
            private_key: value.private_key,
            passphrase: value.passphrase,
        })
    }
    fn delete(&self, id: HostId) -> Result<(), AppError> {
        self.db
            .lock()
            .map_err(|_| failure())?
            .execute("DELETE FROM secrets WHERE id=?1", [id.to_string()])
            .map_err(|_| failure())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
