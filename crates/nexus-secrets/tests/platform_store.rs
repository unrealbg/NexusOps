//! Explicit integration test: exercises the signed-in user's OS keychain.
use nexus_model::{AuthenticationMethod, HostId};
use nexus_secrets::{CredentialInput, EncryptedSecretStore, PlatformKeyProvider, SecretStore};
use std::sync::Arc;

#[test]
#[ignore = "requires an unlocked interactive OS credential store"]
fn platform_keychain_persistence_update_association_and_delete() {
    let directory = tempfile::tempdir().expect("dir");
    let profile = HostId::new().to_string();
    let keys = Arc::new(PlatformKeyProvider::new(&profile));
    let vault_path = directory.path().join("vault.db");
    let store = EncryptedSecretStore::open(&vault_path, keys).expect("vault");
    let password_id = HostId::new();
    let key_id = HostId::new();
    let unavailable_id = HostId::new();
    let canary = format!("nexusops-canary-{}", HostId::new());
    let updated = format!("nexusops-updated-{}", HostId::new());
    let passphrase = format!("nexusops-passphrase-{}", HostId::new());
    let password = CredentialInput {
        password: Some(canary.clone()),
        private_key: None,
        passphrase: None,
    }
    .into_credential(AuthenticationMethod::Password)
    .expect("credential");
    let key = CredentialInput {
        password: None,
        private_key: Some("test-only-private-key-material".into()),
        passphrase: Some(passphrase.clone()),
    }
    .into_credential(AuthenticationMethod::PrivateKey)
    .expect("key credential");
    let result = (|| -> Result<(), nexus_model::AppError> {
        store.put(password_id, &password)?;
        store.put(key_id, &key)?;
        drop(store);

        // A newly opened vault represents an application restart and must use
        // the same master key from Windows Credential Manager.
        let reopened =
            EncryptedSecretStore::open(&vault_path, Arc::new(PlatformKeyProvider::new(&profile)))?;
        assert_eq!(
            reopened
                .get(password_id)?
                .password
                .as_ref()
                .map(|value| value.as_str()),
            Some(canary.as_str())
        );
        let loaded_key = reopened.get(key_id)?;
        assert_eq!(
            loaded_key.private_key.as_ref().map(|value| value.as_str()),
            Some("test-only-private-key-material")
        );
        assert_eq!(
            loaded_key.passphrase.as_ref().map(|value| value.as_str()),
            Some(passphrase.as_str())
        );

        let replacement = CredentialInput {
            password: Some(updated.clone()),
            private_key: None,
            passphrase: None,
        }
        .into_credential(AuthenticationMethod::Password)?;
        let replacement_id = HostId::new();
        reopened.put(replacement_id, &replacement)?;
        reopened.delete(password_id)?;
        assert_eq!(
            reopened
                .get(replacement_id)?
                .password
                .as_ref()
                .map(|value| value.as_str()),
            Some(updated.as_str())
        );
        assert_eq!(
            reopened
                .get(key_id)?
                .passphrase
                .as_ref()
                .map(|value| value.as_str()),
            Some(passphrase.as_str()),
            "credential revisions must remain associated with their host IDs"
        );

        let bytes = std::fs::read(&vault_path).expect("vault bytes");
        for secret in [&canary, &updated, &passphrase] {
            assert!(
                !bytes
                    .windows(secret.len())
                    .any(|window| window == secret.as_bytes()),
                "canary secret was stored as plaintext"
            );
        }
        reopened.delete(replacement_id)?;
        reopened.delete(key_id)?;
        assert!(reopened.get(replacement_id).is_err());
        assert!(reopened.get(key_id).is_err());
        // Retain one encrypted test record so the post-cleanup read below can
        // prove that a missing OS key fails closed.
        reopened.put(unavailable_id, &password)?;
        Ok(())
    })();
    let entry =
        keyring::Entry::new("org.nexusops.desktop", &format!("vault-{profile}")).expect("entry");
    let cleanup = entry.delete_credential();
    result.expect("keychain roundtrip");
    cleanup.expect("remove test key");
    let after_keychain_delete =
        EncryptedSecretStore::open(&vault_path, Arc::new(PlatformKeyProvider::new(&profile)));
    assert!(
        after_keychain_delete
            .expect("vault metadata opens")
            .get(unavailable_id)
            .is_err(),
        "must not silently generate a replacement key"
    );
    assert!(matches!(entry.get_secret(), Err(keyring::Error::NoEntry)));
}
