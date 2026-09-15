use nexus_model::{AppError, AuthenticationMethod, ErrorCode, Host, HostConnectionConfig, HostId};
use rusqlite::{Connection, OptionalExtension, params};
use std::{path::Path, sync::Mutex};

#[derive(Clone)]
pub struct StoredHost {
    pub host: Host,
    pub credential_id: HostId,
}

/// Metadata schema deliberately cannot represent passwords, key files or passphrases.
pub struct HostRepository {
    db: Mutex<Connection>,
}
fn failure() -> AppError {
    AppError::new(
        ErrorCode::Persistence,
        "Cannot read or save local host metadata.",
    )
}
impl HostRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let db = Connection::open(path).map_err(|_| failure())?;
        db.busy_timeout(std::time::Duration::from_secs(3))
            .map_err(|_| failure())?;
        let version: u32 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|_| failure())?;
        if version > 1 {
            return Err(AppError::new(
                ErrorCode::Persistence,
                "This data was created by a newer NexusOps version.",
            ));
        }
        db.execute_batch("PRAGMA journal_mode=WAL; BEGIN IMMEDIATE;
          CREATE TABLE IF NOT EXISTS hosts(id TEXT PRIMARY KEY,display_name TEXT NOT NULL,hostname TEXT NOT NULL,port INTEGER NOT NULL CHECK(port BETWEEN 1 AND 65535),username TEXT NOT NULL,authentication TEXT NOT NULL CHECK(authentication IN ('password','privateKey')),credential_id TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
          PRAGMA user_version=1; COMMIT;").map_err(|_|failure())?;
        Ok(Self { db: Mutex::new(db) })
    }
    pub fn profile_id(&self) -> Result<String, AppError> {
        let db = self.db.lock().map_err(|_| failure())?;
        db.execute(
            "INSERT OR IGNORE INTO settings(key,value) VALUES ('profile_id',?1)",
            [HostId::new().to_string()],
        )
        .map_err(|_| failure())?;
        db.query_row(
            "SELECT value FROM settings WHERE key='profile_id'",
            [],
            |r| r.get(0),
        )
        .map_err(|_| failure())
    }
    pub fn list(&self) -> Result<Vec<Host>, AppError> {
        let db = self.db.lock().map_err(|_| failure())?;
        let mut stmt=db.prepare("SELECT id,display_name,hostname,port,username,authentication,credential_id FROM hosts ORDER BY display_name COLLATE NOCASE,id").map_err(|_|failure())?;
        stmt.query_map([], read_host)
            .map_err(|_| failure())?
            .map(|r| r.map(|s| s.host).map_err(|_| failure()))
            .collect()
    }
    pub fn get(&self, id: HostId) -> Result<StoredHost, AppError> {
        self.db.lock().map_err(|_|failure())?.query_row("SELECT id,display_name,hostname,port,username,authentication,credential_id FROM hosts WHERE id=?1",[id.to_string()],read_host).optional().map_err(|_|failure())?.ok_or_else(||AppError::new(ErrorCode::NotFound,"Host no longer exists."))
    }
    pub fn save(&self, stored: &StoredHost) -> Result<(), AppError> {
        stored.host.connection.validate()?;
        let h = &stored.host;
        let method = match h.connection.authentication {
            AuthenticationMethod::Password => "password",
            AuthenticationMethod::PrivateKey => "privateKey",
        };
        self.db.lock().map_err(|_|failure())?.execute("INSERT INTO hosts(id,display_name,hostname,port,username,authentication,credential_id) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET display_name=excluded.display_name,hostname=excluded.hostname,port=excluded.port,username=excluded.username,authentication=excluded.authentication,credential_id=excluded.credential_id",params![h.id.to_string(),h.display_name,h.connection.hostname,h.connection.port,h.connection.username,method,stored.credential_id.to_string()]).map_err(|_|failure())?;
        Ok(())
    }
    pub fn delete(&self, id: HostId) -> Result<(), AppError> {
        self.db
            .lock()
            .map_err(|_| failure())?
            .execute("DELETE FROM hosts WHERE id=?1", [id.to_string()])
            .map_err(|_| failure())?;
        Ok(())
    }
}
fn read_host(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredHost> {
    let parse = |index| -> rusqlite::Result<HostId> {
        let s: String = r.get(index)?;
        s.parse().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })
    };
    let authentication = match r.get::<_, String>(5)?.as_str() {
        "password" => AuthenticationMethod::Password,
        "privateKey" => AuthenticationMethod::PrivateKey,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(StoredHost {
        host: Host {
            id: parse(0)?,
            display_name: r.get(1)?,
            connection: HostConnectionConfig {
                hostname: r.get(2)?,
                port: r.get(3)?,
                username: r.get(4)?,
                authentication,
            },
        },
        credential_id: parse(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistence_roundtrip_update_multiple_and_delete() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("hosts.db");
        let id = HostId::new();
        let mut stored = StoredHost {
            host: Host {
                id,
                display_name: "Host A".into(),
                connection: HostConnectionConfig {
                    hostname: "localhost".into(),
                    port: 22,
                    username: "root".into(),
                    authentication: AuthenticationMethod::Password,
                },
            },
            credential_id: HostId::new(),
        };
        let profile;
        {
            let repo = HostRepository::open(&path).expect("open");
            profile = repo.profile_id().expect("profile");
            repo.save(&stored).expect("save");
        }
        let repo = HostRepository::open(&path).expect("reopen");
        assert_eq!(repo.profile_id().expect("profile"), profile);
        assert_eq!(repo.get(id).expect("host").host, stored.host);
        stored.host.display_name = "Renamed".into();
        repo.save(&stored).expect("update");
        stored.host.id = HostId::new();
        repo.save(&stored).expect("second");
        assert_eq!(repo.list().expect("list").len(), 2);
        repo.delete(id).expect("delete");
        assert_eq!(
            repo.get(id).err().expect("missing").code,
            ErrorCode::NotFound
        );
        assert_eq!(repo.list().expect("list").len(), 1);
    }
}
