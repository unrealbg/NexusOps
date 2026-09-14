use crate::AuditEvent;
use nexus_model::{AppError, ErrorCode};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

const DEFAULT_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Append-only JSONL with one rotated backup. Use one instance per desktop
/// process (shared by Arc); the lock serializes writes and rotation in-process.
/// Audit files are diagnostic records, not a tamper-evident security ledger.
pub struct AuditLog {
    path: PathBuf,
    max_bytes: u64,
    write_lock: Mutex<()>,
}

impl AuditLog {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> {
        Self::open_with_limit(path, DEFAULT_MAX_BYTES)
    }

    pub fn open_with_limit(path: impl AsRef<Path>, max_bytes: u64) -> Result<Self, AppError> {
        if max_bytes < 1024 {
            return Err(AppError::new(
                ErrorCode::Validation,
                "The audit file limit must be at least 1 KiB.",
            ));
        }
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|_| storage_error())?;
        }
        open_append(&path)?;
        Ok(Self {
            path,
            max_bytes,
            write_lock: Mutex::new(()),
        })
    }

    /// Persist an event and flush it to storage. Failures propagate to the caller;
    /// callers must surface loss of auditing instead of silently dropping it.
    pub fn record(&self, event: &AuditEvent) -> Result<(), AppError> {
        event.validate()?;
        let mut encoded = serde_json::to_vec(event).map_err(|_| storage_error())?;
        encoded.push(b'\n');
        if encoded.len() as u64 > self.max_bytes {
            return Err(AppError::new(
                ErrorCode::Validation,
                "The audit event exceeds the local record limit.",
            ));
        }
        let _guard = self.write_lock.lock().map_err(|_| storage_error())?;
        let length = fs::metadata(&self.path).map_err(|_| storage_error())?.len();
        if length.saturating_add(encoded.len() as u64) > self.max_bytes {
            let mut backup_name = self.path.as_os_str().to_os_string();
            backup_name.push(".1");
            let backup = PathBuf::from(backup_name);
            match fs::remove_file(&backup) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(storage_error()),
            }
            fs::rename(&self.path, backup).map_err(|_| storage_error())?;
        }
        let mut file = open_append(&self.path)?;
        file.write_all(&encoded).map_err(|_| storage_error())?;
        file.sync_data().map_err(|_| storage_error())?;
        Ok(())
    }
}

fn open_append(path: &Path) -> Result<File, AppError> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|_| storage_error())
}

fn storage_error() -> AppError {
    AppError::new(
        ErrorCode::Persistence,
        "The local audit record could not be saved.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuditActor, AuditOutcome};
    use nexus_model::{HostId, Operation, OperationRisk};

    fn event() -> AuditEvent {
        AuditEvent::new(
            HostId::new(),
            Operation {
                id: "test-operation-01".into(),
                kind: "host.discovery".into(),
                risk: OperationRisk::ReadOnly,
            },
            AuditActor::User,
            AuditOutcome::Success,
            12,
        )
    }

    #[test]
    fn persists_metadata_only_and_appends_across_reopens() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("audit/events.jsonl");
        let record = event();
        AuditLog::open(&path)
            .expect("open")
            .record(&record)
            .expect("record");
        AuditLog::open(&path)
            .expect("reopen")
            .record(&record)
            .expect("append");
        let contents = fs::read_to_string(&path).expect("read log");
        assert_eq!(contents.lines().count(), 2);
        let value: serde_json::Value =
            serde_json::from_str(contents.lines().next().expect("line")).expect("JSON");
        let object = value.as_object().expect("event object");
        assert_eq!(object.len(), 6);
        assert_eq!(value["durationMs"], 12);
        assert_eq!(value["operation"]["kind"], "host.discovery");
        for forbidden in [
            "password",
            "passphrase",
            "privateKey",
            "command",
            "output",
            "stderr",
            "stdout",
            "error",
        ] {
            assert!(!contents.contains(forbidden));
        }
    }

    #[test]
    fn rejects_freeform_or_sensitive_payloads_in_metadata() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("events.jsonl");
        let log = AuditLog::open(&path).expect("open");
        let mut record = event();
        record.operation.kind = "ssh password=super secret".into();
        assert!(log.record(&record).is_err());
        assert_eq!(fs::metadata(path).expect("metadata").len(), 0);
        let mut value = serde_json::to_value(event()).expect("event JSON");
        value["password"] = "secret".into();
        assert!(serde_json::from_value::<AuditEvent>(value).is_err());
    }

    #[test]
    fn rotation_bounds_both_files_and_keeps_json_lines_complete() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("events.jsonl");
        let log = AuditLog::open_with_limit(&path, 1024).expect("open");
        for _ in 0..20 {
            log.record(&event()).expect("record");
        }
        for file in [path, directory.path().join("events.jsonl.1")] {
            let content = fs::read_to_string(&file).expect("rotated log");
            assert!(content.len() <= 1024);
            assert!(content.ends_with('\n'));
            for line in content.lines() {
                serde_json::from_str::<AuditEvent>(line).expect("complete event");
            }
        }
    }

    #[test]
    fn storage_failure_is_not_silently_ignored() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("events.jsonl");
        let log = AuditLog::open(&path).expect("open");
        fs::remove_file(&path).expect("remove");
        fs::create_dir(&path).expect("replace with directory");
        let error = log.record(&event()).expect_err("I/O failure");
        assert_eq!(error.code, ErrorCode::Persistence);
    }
}
