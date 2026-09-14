use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
struct TestKey;
impl KeyProvider for TestKey {
    fn key(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        Ok(Zeroizing::new(vec![7; 32]))
    }
}
struct Locked;
impl KeyProvider for Locked {
    fn key(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        Err(failure())
    }
}
fn credential() -> Credential {
    Credential {
        password: Some(Zeroizing::new("NEVER_PLAINTEXT".into())),
        private_key: None,
        passphrase: None,
    }
}
#[test]
fn encrypted_roundtrip_delete_and_no_plaintext() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("vault.db");
    let id = HostId::new();
    {
        let store = EncryptedSecretStore::open(&path, Arc::new(TestKey)).expect("vault");
        store.put(id, &credential()).expect("put");
        assert!(store.put(id, &credential()).is_err());
    }
    assert!(
        !std::fs::read(&path)
            .expect("read")
            .windows(b"NEVER_PLAINTEXT".len())
            .any(|w| w == b"NEVER_PLAINTEXT")
    );
    let store = EncryptedSecretStore::open(&path, Arc::new(TestKey)).expect("reopen");
    assert_eq!(
        store
            .get(id)
            .expect("get")
            .password
            .as_deref()
            .map(String::as_str),
        Some("NEVER_PLAINTEXT")
    );
    store.delete(id).expect("delete");
    assert!(store.get(id).is_err());
}
#[test]
fn locked_storage_and_tampered_ciphertext_fail_closed() {
    let store = EncryptedSecretStore::open(":memory:", Arc::new(Locked)).expect("vault");
    assert!(store.put(HostId::new(), &credential()).is_err());
    let store = EncryptedSecretStore::open(":memory:", Arc::new(TestKey)).expect("vault");
    let id = HostId::new();
    store.put(id, &credential()).expect("put");
    store
        .db
        .lock()
        .expect("lock")
        .execute("UPDATE secrets SET ciphertext=zeroblob(32)", [])
        .expect("tamper");
    assert!(store.get(id).is_err());
}

#[derive(Default)]
struct RecoverableTestKey {
    present: AtomicBool,
    initializations: AtomicUsize,
}
impl KeyProvider for RecoverableTestKey {
    fn key(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if self.present.load(Ordering::SeqCst) {
            Ok(Zeroizing::new(vec![19; 32]))
        } else {
            Err(failure())
        }
    }
    fn initialize(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.initializations.fetch_add(1, Ordering::SeqCst);
        self.present.store(true, Ordering::SeqCst);
        self.key()
    }
}

#[test]
fn missing_master_never_regenerates_even_after_last_credential_deleted() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("vault.db");
    let keys = Arc::new(RecoverableTestKey::default());
    let id = HostId::new();
    {
        let store = EncryptedSecretStore::open(&path, keys.clone()).expect("vault");
        assert!(store.get(id).is_err());
        assert_eq!(keys.initializations.load(Ordering::SeqCst), 0);
        store.put(id, &credential()).expect("initialize vault");
        keys.present.store(false, Ordering::SeqCst);
        assert_eq!(
            store.get(id).err().expect("missing master").code,
            ErrorCode::SecureStorage
        );
        assert_eq!(
            store
                .put(HostId::new(), &credential())
                .expect_err("cannot rekey")
                .code,
            ErrorCode::SecureStorage
        );
        assert_eq!(keys.initializations.load(Ordering::SeqCst), 1);
        store.delete(id).expect("delete ciphertext");
    }
    let store =
        EncryptedSecretStore::open(path, keys.clone()).expect("reopen empty initialized vault");
    assert!(store.put(HostId::new(), &credential()).is_err());
    assert_eq!(keys.initializations.load(Ordering::SeqCst), 1);
}

#[test]
fn migrates_nonempty_vault_to_initialized_without_rekeying() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("vault.db");
    let keys = Arc::new(RecoverableTestKey::default());
    {
        let store = EncryptedSecretStore::open(&path, keys.clone()).expect("vault");
        store.put(HostId::new(), &credential()).expect("put");
        store
            .db
            .lock()
            .expect("database")
            .execute("DROP TABLE vault_state", [])
            .expect("simulate prior schema");
    }
    keys.present.store(false, Ordering::SeqCst);
    let store = EncryptedSecretStore::open(path, keys.clone()).expect("migrate");
    assert!(store.put(HostId::new(), &credential()).is_err());
    assert_eq!(keys.initializations.load(Ordering::SeqCst), 1);
}

#[test]
fn maximum_escaped_password_roundtrips_and_oversized_fields_are_rejected() {
    let store = EncryptedSecretStore::open(":memory:", Arc::new(TestKey)).expect("vault");
    let id = HostId::new();
    let value = Credential {
        password: Some(Zeroizing::new("\u{1}".repeat(65_536))),
        private_key: None,
        passphrase: None,
    };
    store.put(id, &value).expect("valid maximum-sized password");
    let loaded = store
        .get(id)
        .expect("escaped JSON exceeds 200 KiB but is valid");
    assert_eq!(loaded.password.as_deref(), value.password.as_deref());
    let oversized = Credential {
        password: Some(Zeroizing::new("x".repeat(65_537))),
        private_key: None,
        passphrase: None,
    };
    assert_eq!(
        store
            .put(HostId::new(), &oversized)
            .expect_err("too large")
            .code,
        ErrorCode::Validation
    );
}

#[test]
fn authenticated_revision_id_prevents_ciphertext_swapping() {
    let store = EncryptedSecretStore::open(":memory:", Arc::new(TestKey)).expect("vault");
    let first = HostId::new();
    let second = HostId::new();
    store.put(first, &credential()).expect("put");
    store.db.lock().expect("database").execute(
            "INSERT INTO secrets(id,nonce,ciphertext) SELECT ?1,nonce,ciphertext FROM secrets WHERE id=?2",
            params![second.to_string(), first.to_string()],
        ).expect("copy ciphertext");
    assert!(store.get(first).is_ok());
    assert!(store.get(second).is_err());
}

#[test]
fn initial_aes_gcm_010_ciphertext_remains_readable_after_marker_migration() {
    // Generated using aes-gcm 0.10.3, key=[7;32], nonce=[1;12], the original
    // serialized credential schema and nexusops:v1:<revision-id> associated data.
    const LEGACY_CIPHERTEXT: &[u8] = &[
        13, 195, 249, 214, 227, 204, 155, 97, 163, 176, 254, 27, 82, 91, 21, 86, 201, 152, 109,
        217, 107, 242, 88, 97, 61, 69, 97, 78, 68, 196, 68, 210, 180, 211, 124, 213, 215, 118, 252,
        162, 100, 28, 61, 103, 173, 82, 204, 233, 239, 104, 154, 114, 53, 204, 158, 44, 199, 96,
        157, 102, 163, 78, 233, 232, 157, 213, 189, 55, 131, 88, 228, 10, 116, 176, 186, 124, 195,
        9, 113, 118,
    ];
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("legacy-vault.db");
    let id: HostId = "00000000-0000-0000-0000-000000000000"
        .parse()
        .expect("revision ID");
    {
        let database = Connection::open(&path).expect("legacy database");
        database.execute_batch("CREATE TABLE secrets(id TEXT PRIMARY KEY, nonce BLOB NOT NULL, ciphertext BLOB NOT NULL);").expect("legacy schema");
        database
            .execute(
                "INSERT INTO secrets(id,nonce,ciphertext) VALUES (?1,?2,?3)",
                params![id.to_string(), &[1_u8; 12], LEGACY_CIPHERTEXT],
            )
            .expect("legacy row");
    }
    let store = EncryptedSecretStore::open(path, Arc::new(TestKey)).expect("migrate vault");
    let loaded = store
        .get(id)
        .expect("decrypt legacy payload with aes-gcm 0.11");
    assert_eq!(
        loaded.password.as_ref().map(|value| value.as_str()),
        Some("legacy-secret")
    );
    let initialized: bool = store
        .db
        .lock()
        .expect("database")
        .query_row(
            "SELECT initialized FROM vault_state WHERE id=1",
            [],
            |row| row.get(0),
        )
        .expect("migration marker");
    assert!(initialized);
}
