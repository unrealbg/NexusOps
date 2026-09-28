use std::{net::IpAddr, path::Path, sync::Mutex, time::Duration};

use nexus_model::{AppError, ErrorCode, HostFingerprint, HostKeyChallenge};
use rusqlite::{Connection, OptionalExtension, params};

/// Persisted TOFU pins scoped to the canonical configured hostname and SSH port.
/// Trust never replaces an existing pin, including during competing trust requests.
pub struct KnownHosts {
    connection: Mutex<Connection>,
}

impl KnownHosts {
    /// Canonical endpoint spelling used by persisted pins and blocked challenges.
    pub fn canonical_hostname(hostname: &str, port: u16) -> Result<String, AppError> {
        canonical_endpoint(hostname, port)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let connection = Connection::open(path).map_err(database_error)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(database_error)?;
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS ssh_host_keys (
                hostname TEXT NOT NULL,
                port INTEGER NOT NULL CHECK(port BETWEEN 1 AND 65535),
                algorithm TEXT NOT NULL,
                fingerprint TEXT NOT NULL,
                PRIMARY KEY(hostname, port)
            );",
            )
            .map_err(database_error)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Verify before authentication. Unknown and changed keys return distinct challenges.
    pub fn verify(
        &self,
        hostname: &str,
        port: u16,
        fingerprint: &HostFingerprint,
    ) -> Result<(), AppError> {
        let hostname = canonical_endpoint(hostname, port)?;
        validate_fingerprint(&fingerprint.algorithm, &fingerprint.sha256)?;
        let connection = self.connection.lock().map_err(|_| poisoned_database())?;
        let stored = read_pin(&connection, &hostname, port)?;
        compare_pin(hostname, port, fingerprint, stored)
    }

    /// Read bounded, validated local pin metadata; absence is distinct from invalid storage.
    pub fn fingerprint(
        &self,
        hostname: &str,
        port: u16,
    ) -> Result<Option<HostFingerprint>, AppError> {
        let hostname = canonical_endpoint(hostname, port)?;
        let connection = self.connection.lock().map_err(|_| poisoned_database())?;
        let pin = read_pin(&connection, &hostname, port)?;
        if let Some(pin) = &pin {
            validate_fingerprint(&pin.algorithm, &pin.sha256).map_err(|_| {
                AppError::new(
                    ErrorCode::Persistence,
                    "The trusted SSH host key store is invalid.",
                )
            })?;
        }
        Ok(pin)
    }

    /// Commit an explicitly accepted challenge. The application service is responsible
    /// for binding this value to the currently pending handshake challenge.
    pub fn trust(&self, challenge: &HostKeyChallenge) -> Result<(), AppError> {
        let hostname = canonical_endpoint(&challenge.hostname, challenge.port)?;
        validate_fingerprint(&challenge.algorithm, &challenge.fingerprint)?;
        if challenge.previous_fingerprint.is_some() {
            return Err(AppError::new(
                ErrorCode::ChangedHostKey,
                "A changed SSH host key cannot be trusted through this action.",
            ));
        }
        let mut connection = self.connection.lock().map_err(|_| poisoned_database())?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let stored = read_pin(&transaction, &hostname, challenge.port)?;
        let fingerprint = HostFingerprint {
            algorithm: challenge.algorithm.clone(),
            sha256: challenge.fingerprint.clone(),
        };
        if stored.is_some() {
            compare_pin(hostname, challenge.port, &fingerprint, stored)?;
        } else {
            transaction.execute(
                "INSERT INTO ssh_host_keys(hostname, port, algorithm, fingerprint) VALUES (?1, ?2, ?3, ?4)",
                params![hostname, challenge.port, challenge.algorithm, challenge.fingerprint],
            ).map_err(database_error)?;
        }
        transaction.commit().map_err(database_error)
    }

    /// Compare-and-swap a persisted endpoint pin. This is application-only authority;
    /// no renderer command accepts fingerprint or endpoint arguments for mutation.
    pub fn rotate(
        &self,
        hostname: &str,
        port: u16,
        expected_current: &HostFingerprint,
        replacement: &HostFingerprint,
    ) -> Result<(), AppError> {
        let hostname = canonical_endpoint(hostname, port)?;
        validate_fingerprint(&expected_current.algorithm, &expected_current.sha256)?;
        validate_fingerprint(&replacement.algorithm, &replacement.sha256)?;
        if expected_current == replacement {
            return Err(rotation_conflict());
        }
        let mut connection = self.connection.lock().map_err(|_| poisoned_database())?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let stored = read_pin(&transaction, &hostname, port)?;
        if let Some(pin) = &stored {
            validate_fingerprint(&pin.algorithm, &pin.sha256).map_err(|_| {
                AppError::new(
                    ErrorCode::Persistence,
                    "The trusted SSH host key store is invalid.",
                )
            })?;
        }
        if stored.as_ref() != Some(expected_current) {
            return Err(rotation_conflict());
        }
        let updated = transaction
            .execute(
                "UPDATE ssh_host_keys SET algorithm=?1, fingerprint=?2
                 WHERE hostname=?3 AND port=?4 AND algorithm=?5 AND fingerprint=?6",
                params![
                    replacement.algorithm,
                    replacement.sha256,
                    hostname,
                    port,
                    expected_current.algorithm,
                    expected_current.sha256,
                ],
            )
            .map_err(database_error)?;
        if updated != 1 {
            return Err(rotation_conflict());
        }
        transaction.commit().map_err(database_error)
    }
}

fn rotation_conflict() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "The trusted SSH host key changed. Review a new rotation plan.",
    )
}

fn read_pin(
    connection: &Connection,
    hostname: &str,
    port: u16,
) -> Result<Option<HostFingerprint>, AppError> {
    connection
        .query_row(
            "SELECT algorithm, fingerprint FROM ssh_host_keys WHERE hostname = ?1 AND port = ?2",
            params![hostname, port],
            |row| {
                Ok(HostFingerprint {
                    algorithm: row.get(0)?,
                    sha256: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(database_error)
}

fn compare_pin(
    hostname: String,
    port: u16,
    fingerprint: &HostFingerprint,
    stored: Option<HostFingerprint>,
) -> Result<(), AppError> {
    if stored.as_ref().is_some_and(|pin| {
        pin.algorithm == fingerprint.algorithm && pin.sha256 == fingerprint.sha256
    }) {
        return Ok(());
    }
    let changed = stored.is_some();
    Err(AppError {
        code: if changed { ErrorCode::ChangedHostKey } else { ErrorCode::UnknownHostKey },
        message: if changed {
            "The SSH host key has changed. Connection blocked; verify the server identity independently."
        } else {
            "This SSH host is unknown. Verify its fingerprint and explicitly trust it before connecting."
        }.into(),
        host_key: Some(Box::new(HostKeyChallenge {
            hostname,
            port,
            algorithm: fingerprint.algorithm.clone(),
            fingerprint: fingerprint.sha256.clone(),
            previous_fingerprint: stored.map(|pin| pin.sha256),
        })),
    })
}

pub(crate) fn canonical_endpoint(hostname: &str, port: u16) -> Result<String, AppError> {
    let hostname = hostname.trim().trim_end_matches('.');
    let address = hostname.parse::<IpAddr>();
    let valid_dns = !hostname.is_empty()
        && hostname.len() <= 253
        && hostname.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    if port == 0 || (address.is_err() && !valid_dns) {
        return Err(AppError::new(
            ErrorCode::Validation,
            "A valid hostname and SSH port are required.",
        ));
    }
    Ok(address
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| hostname.to_ascii_lowercase()))
}

fn validate_fingerprint(algorithm: &str, fingerprint: &str) -> Result<(), AppError> {
    let digest = fingerprint.strip_prefix("SHA256:").unwrap_or("");
    if algorithm.is_empty()
        || algorithm.len() > 128
        || !algorithm
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-@._".contains(&byte))
        || digest.len() != 43
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"+/".contains(&byte))
    {
        return Err(AppError::new(
            ErrorCode::Validation,
            "Invalid SSH host fingerprint.",
        ));
    }
    Ok(())
}

fn database_error(error: rusqlite::Error) -> AppError {
    tracing::error!(component = "known_hosts", sqlite_code = ?error.sqlite_error_code(), "Host key storage failed");
    AppError::new(
        ErrorCode::Persistence,
        "The trusted SSH host keys could not be read or saved.",
    )
}

fn poisoned_database() -> AppError {
    AppError::new(
        ErrorCode::Persistence,
        "The trusted SSH host key store is unavailable.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(ch: char) -> HostFingerprint {
        HostFingerprint {
            algorithm: "ssh-ed25519".into(),
            sha256: format!("SHA256:{}", ch.to_string().repeat(43)),
        }
    }

    #[test]
    fn persisted_pin_read_rejects_tampered_values_as_storage_errors() {
        let store = KnownHosts::open(":memory:").unwrap();
        assert_eq!(store.fingerprint("host", 22).unwrap(), None);
        let valid = fingerprint('A');
        for (algorithm, digest) in [
            ("x".repeat(129), valid.sha256.clone()),
            ("ssh\ned25519".into(), valid.sha256.clone()),
            ("ssh\u{202e}ed25519".into(), valid.sha256.clone()),
            (
                valid.algorithm.clone(),
                format!("SHA512:{}", "A".repeat(43)),
            ),
            (
                valid.algorithm.clone(),
                format!("SHA256:{}", "A".repeat(44)),
            ),
            (
                valid.algorithm.clone(),
                format!("SHA256:{}", "!".repeat(43)),
            ),
        ] {
            store
                .connection
                .lock()
                .unwrap()
                .execute(
                    "INSERT OR REPLACE INTO ssh_host_keys VALUES ('host',22,?1,?2)",
                    params![algorithm, digest],
                )
                .unwrap();
            let error = store.fingerprint("host", 22).unwrap_err();
            assert_eq!(error.code, ErrorCode::Persistence);
            assert_eq!(error.message, "The trusted SSH host key store is invalid.");
            assert!(error.host_key.is_none());
        }
        store
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE ssh_host_keys SET algorithm=?1, fingerprint=?2",
                params![valid.algorithm, valid.sha256],
            )
            .unwrap();
        assert_eq!(store.fingerprint("HOST.", 22).unwrap(), Some(valid));
    }

    #[test]
    fn unknown_requires_trust_and_pin_survives_reopen() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("known_hosts.db");
        let store = KnownHosts::open(&path).expect("store");
        let error = store
            .verify("SERVER.Example.", 22, &fingerprint('A'))
            .expect_err("unknown");
        assert_eq!(error.code, ErrorCode::UnknownHostKey);
        let challenge = error.host_key.expect("challenge");
        assert_eq!(challenge.hostname, "server.example");
        store.trust(&challenge).expect("trust");
        drop(store);
        KnownHosts::open(path)
            .expect("reopen")
            .verify("server.example", 22, &fingerprint('A'))
            .expect("trusted");
    }

    #[test]
    fn changed_key_and_stale_challenge_cannot_replace_pin() {
        let store = KnownHosts::open(":memory:").expect("store");
        let first = store
            .verify("host", 22, &fingerprint('A'))
            .expect_err("unknown")
            .host_key
            .expect("challenge");
        let stale = store
            .verify("host", 22, &fingerprint('B'))
            .expect_err("unknown")
            .host_key
            .expect("challenge");
        store.trust(&first).expect("first trust");
        assert_eq!(
            store.trust(&stale).expect_err("stale").code,
            ErrorCode::ChangedHostKey
        );
        let changed = store
            .verify("host", 22, &fingerprint('B'))
            .expect_err("changed");
        assert_eq!(changed.code, ErrorCode::ChangedHostKey);
        assert_eq!(
            store
                .trust(&changed.host_key.expect("challenge"))
                .expect_err("cannot trust rotation")
                .code,
            ErrorCode::ChangedHostKey
        );
        store
            .verify("host", 22, &fingerprint('A'))
            .expect("original intact");
        store.trust(&first).expect("idempotent trust");
    }

    #[test]
    fn pins_are_scoped_to_port_hostname_and_algorithm() {
        let store = KnownHosts::open(":memory:").expect("store");
        let challenge = store
            .verify("host", 22, &fingerprint('A'))
            .expect_err("unknown")
            .host_key
            .expect("challenge");
        store.trust(&challenge).expect("trust");
        assert_eq!(
            store
                .verify("host", 2222, &fingerprint('A'))
                .expect_err("different port")
                .code,
            ErrorCode::UnknownHostKey
        );
        assert_eq!(
            store
                .verify("other", 22, &fingerprint('A'))
                .expect_err("different hostname")
                .code,
            ErrorCode::UnknownHostKey
        );
        let mut different_algorithm = fingerprint('A');
        different_algorithm.algorithm = "ecdsa-sha2-nistp256".into();
        assert_eq!(
            store
                .verify("host", 22, &different_algorithm)
                .expect_err("different algorithm")
                .code,
            ErrorCode::ChangedHostKey
        );
        assert_eq!(
            canonical_endpoint("2001:0db8::1", 22).expect("IP"),
            "2001:db8::1"
        );
    }

    #[test]
    fn invalid_pins_fail_closed() {
        let store = KnownHosts::open(":memory:").expect("store");
        assert_eq!(
            store
                .verify("host", 0, &fingerprint('A'))
                .expect_err("port")
                .code,
            ErrorCode::Validation
        );
        assert_eq!(
            store
                .verify("host", 22, &fingerprint('!'))
                .expect_err("digest")
                .code,
            ErrorCode::Validation
        );
        for hostname in ["ssh://host", "host;id", "-host", "a..b"] {
            assert_eq!(
                store
                    .verify(hostname, 22, &fingerprint('A'))
                    .expect_err("invalid endpoint")
                    .code,
                ErrorCode::Validation
            );
        }
    }

    #[test]
    fn racing_separate_connections_cannot_replace_each_others_pin() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("pins.db");
        let first = KnownHosts::open(&path).expect("first connection");
        let second = KnownHosts::open(&path).expect("second connection");
        let first_challenge = first
            .verify("host", 22, &fingerprint('A'))
            .expect_err("unknown")
            .host_key
            .expect("challenge");
        let second_challenge = second
            .verify("host", 22, &fingerprint('B'))
            .expect_err("unknown")
            .host_key
            .expect("challenge");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let race = std::thread::spawn({
            let barrier = barrier.clone();
            move || {
                barrier.wait();
                first.trust(&first_challenge)
            }
        });
        barrier.wait();
        let second_result = second.trust(&second_challenge);
        let first_result = race.join().expect("thread");
        assert_ne!(first_result.is_ok(), second_result.is_ok());
        let rejection = first_result
            .err()
            .or_else(|| second_result.err())
            .expect("one rejection");
        assert_eq!(rejection.code, ErrorCode::ChangedHostKey);
    }

    #[test]
    fn rotation_is_exact_endpoint_cas_and_persists_after_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pins.db");
        let store = KnownHosts::open(&path).unwrap();
        let old = fingerprint('A');
        let next = fingerprint('B');
        store
            .trust(
                &store
                    .verify("HOST.", 22, &old)
                    .unwrap_err()
                    .host_key
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            store.rotate("other", 22, &old, &next).unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(
            store.rotate("host", 2222, &old, &next).unwrap_err().code,
            ErrorCode::Conflict
        );
        let mut wrong_algorithm = old.clone();
        wrong_algorithm.algorithm = "ecdsa-sha2-nistp256".into();
        assert_eq!(
            store
                .rotate("host", 22, &wrong_algorithm, &next)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            store
                .rotate("host", 22, &fingerprint('C'), &next)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            store.rotate("host", 22, &old, &old).unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(store.fingerprint("host", 22).unwrap(), Some(old.clone()));
        store.rotate("HOST.", 22, &old, &next).unwrap();
        assert_eq!(
            store
                .rotate("host", 22, &old, &fingerprint('C'))
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(store.fingerprint("host", 22).unwrap(), Some(next.clone()));
        assert_eq!(store.fingerprint("other", 22).unwrap(), None);
        assert_eq!(store.fingerprint("host", 2222).unwrap(), None);
        drop(store);
        assert_eq!(
            KnownHosts::open(&path)
                .unwrap()
                .fingerprint("host", 22)
                .unwrap(),
            Some(next)
        );
    }

    #[test]
    fn rotation_allows_algorithm_or_digest_change_but_rejects_invalid_values() {
        let store = KnownHosts::open(":memory:").unwrap();
        let old = fingerprint('A');
        store
            .trust(
                &store
                    .verify("host", 22, &old)
                    .unwrap_err()
                    .host_key
                    .unwrap(),
            )
            .unwrap();
        let mut algorithm_only = old.clone();
        algorithm_only.algorithm = "ecdsa-sha2-nistp256".into();
        assert_eq!(
            store.verify("host", 22, &algorithm_only).unwrap_err().code,
            ErrorCode::ChangedHostKey
        );
        store.rotate("host", 22, &old, &algorithm_only).unwrap();
        let digest_only = HostFingerprint {
            algorithm: algorithm_only.algorithm.clone(),
            sha256: fingerprint('B').sha256,
        };
        store
            .rotate("host", 22, &algorithm_only, &digest_only)
            .unwrap();
        for bad in [
            HostFingerprint {
                algorithm: "bad\nvalue".into(),
                sha256: digest_only.sha256.clone(),
            },
            HostFingerprint {
                algorithm: "ssh-ed25519".into(),
                sha256: "SHA256:short".into(),
            },
        ] {
            assert_eq!(
                store.rotate("host", 22, &bad, &old).unwrap_err().code,
                ErrorCode::Validation
            );
            assert_eq!(
                store
                    .rotate("host", 22, &digest_only, &bad)
                    .unwrap_err()
                    .code,
                ErrorCode::Validation
            );
        }
        assert_eq!(store.fingerprint("host", 22).unwrap(), Some(digest_only));
    }

    #[test]
    fn rotation_fails_closed_for_missing_or_corrupt_storage() {
        let store = KnownHosts::open(":memory:").unwrap();
        let old = fingerprint('A');
        let next = fingerprint('B');
        assert_eq!(
            store.rotate("host", 22, &old, &next).unwrap_err().code,
            ErrorCode::Conflict
        );
        store
            .trust(
                &store
                    .verify("host", 22, &old)
                    .unwrap_err()
                    .host_key
                    .unwrap(),
            )
            .unwrap();
        store
            .connection
            .lock()
            .unwrap()
            .execute("UPDATE ssh_host_keys SET fingerprint='corrupt'", [])
            .unwrap();
        let error = store.rotate("host", 22, &old, &next).unwrap_err();
        assert_eq!(error.code, ErrorCode::Persistence);
        assert!(!error.message.contains("corrupt"));
    }

    #[test]
    fn racing_rotations_on_separate_connections_have_one_winner() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pins.db");
        let first = KnownHosts::open(&path).unwrap();
        let second = KnownHosts::open(&path).unwrap();
        let old = fingerprint('A');
        first
            .trust(
                &first
                    .verify("host", 22, &old)
                    .unwrap_err()
                    .host_key
                    .unwrap(),
            )
            .unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let race = std::thread::spawn({
            let barrier = barrier.clone();
            let old = old.clone();
            move || {
                barrier.wait();
                first.rotate("host", 22, &old, &fingerprint('B'))
            }
        });
        barrier.wait();
        let second_result = second.rotate("host", 22, &old, &fingerprint('C'));
        let first_result = race.join().unwrap();
        assert_ne!(first_result.is_ok(), second_result.is_ok());
        assert_eq!(
            first_result
                .err()
                .or_else(|| second_result.err())
                .unwrap()
                .code,
            ErrorCode::Conflict
        );
        let winner = KnownHosts::open(&path)
            .unwrap()
            .fingerprint("host", 22)
            .unwrap()
            .unwrap();
        assert!(winner == fingerprint('B') || winner == fingerprint('C'));
    }
}
