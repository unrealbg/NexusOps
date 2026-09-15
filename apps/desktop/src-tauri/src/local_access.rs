use nexus_model::{
    AppError, ErrorCode, LocalGrantId, LocalGrantKind, LocalSelectionGrant, LocalSelectionItem,
};
use nexus_sftp::LocalItem;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

const GRANT_LIFETIME: Duration = Duration::from_secs(10 * 60);
const GRANT_CAP: usize = 64;
const FILE_CAP: usize = 32;

enum GrantValue {
    Upload(Vec<LocalItem>),
    DownloadDirectory(PathBuf),
}
struct StoredGrant {
    value: GrantValue,
    expires: Instant,
}

#[derive(Default)]
pub struct LocalAccessService {
    grants: Mutex<HashMap<LocalGrantId, StoredGrant>>,
}

impl LocalAccessService {
    pub async fn choose_upload_files(&self) -> Result<Option<LocalSelectionGrant>, AppError> {
        let Some(handles) = rfd::AsyncFileDialog::new()
            .set_title("Choose files to upload")
            .pick_files()
            .await
        else {
            return Ok(None);
        };
        if handles.is_empty() || handles.len() > FILE_CAP {
            return Err(local_error("Choose between 1 and 32 regular files."));
        }
        let mut items = Vec::with_capacity(handles.len());
        let mut views = Vec::with_capacity(handles.len());
        for handle in handles {
            let path = handle.path().to_path_buf();
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|_| local_error("A selected local file is unavailable."))?;
            if !metadata.file_type().is_file() || is_reparse(&metadata) {
                return Err(local_error(
                    "Uploads accept only regular non-reparse files.",
                ));
            }
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    local_error("A selected filename cannot be represented losslessly.")
                })?
                .to_owned();
            nexus_sftp::validate_child_name(&name)?;
            views.push(LocalSelectionItem {
                display_name: nexus_sftp::display_name(&name),
                size_bytes: Some(metadata.len().to_string()),
            });
            items.push(LocalItem {
                path,
                display_name: name,
                size: metadata.len(),
                modified: metadata.modified().ok(),
            });
        }
        self.insert(
            LocalGrantKind::UploadFiles,
            views,
            GrantValue::Upload(items),
        )
    }

    pub async fn choose_download_directory(&self) -> Result<Option<LocalSelectionGrant>, AppError> {
        let Some(handle) = rfd::AsyncFileDialog::new()
            .set_title("Choose download destination")
            .pick_folder()
            .await
        else {
            return Ok(None);
        };
        let path = handle.path().to_path_buf();
        nexus_sftp::validate_local_directory(&path)?;
        self.insert(
            LocalGrantKind::DownloadDirectory,
            vec![LocalSelectionItem {
                display_name: "Selected destination".into(),
                size_bytes: None,
            }],
            GrantValue::DownloadDirectory(path),
        )
    }

    pub fn consume_upload(&self, id: LocalGrantId) -> Result<Vec<LocalItem>, AppError> {
        match self.consume(id)? {
            GrantValue::Upload(items) => Ok(items),
            _ => Err(local_error("The local grant has the wrong scope.")),
        }
    }
    pub fn consume_download_directory(&self, id: LocalGrantId) -> Result<PathBuf, AppError> {
        match self.consume(id)? {
            GrantValue::DownloadDirectory(path) => Ok(path),
            _ => Err(local_error("The local grant has the wrong scope.")),
        }
    }
    pub fn revoke_all(&self) {
        if let Ok(mut grants) = self.grants.lock() {
            grants.clear();
        }
    }

    fn insert(
        &self,
        kind: LocalGrantKind,
        items: Vec<LocalSelectionItem>,
        value: GrantValue,
    ) -> Result<Option<LocalSelectionGrant>, AppError> {
        let id = LocalGrantId::new();
        let mut grants = self
            .grants
            .lock()
            .map_err(|_| local_error("Local access grants are unavailable."))?;
        grants.retain(|_, grant| grant.expires > Instant::now());
        if grants.len() >= GRANT_CAP {
            return Err(local_error(
                "Too many local selections are awaiting approval.",
            ));
        }
        grants.insert(
            id,
            StoredGrant {
                value,
                expires: Instant::now() + GRANT_LIFETIME,
            },
        );
        Ok(Some(LocalSelectionGrant {
            id,
            kind,
            items,
            expires_at: (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339(),
        }))
    }
    fn consume(&self, id: LocalGrantId) -> Result<GrantValue, AppError> {
        let mut grants = self
            .grants
            .lock()
            .map_err(|_| local_error("Local access grants are unavailable."))?;
        let grant = grants.remove(&id).ok_or_else(|| {
            local_error("The local selection grant is missing or was already used.")
        })?;
        if Instant::now() > grant.expires {
            return Err(local_error("The local selection grant expired."));
        }
        Ok(grant.value)
    }
}

#[cfg(windows)]
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}
fn local_error(message: &'static str) -> AppError {
    AppError::new(ErrorCode::LocalAccess, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_are_typed_one_shot_bounded_and_expiring() {
        let service = LocalAccessService::default();
        let directory = tempfile::tempdir().unwrap();
        let view = vec![LocalSelectionItem {
            display_name: "Destination".into(),
            size_bytes: None,
        }];
        let grant = service
            .insert(
                LocalGrantKind::DownloadDirectory,
                view.clone(),
                GrantValue::DownloadDirectory(directory.path().to_path_buf()),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            service.consume_upload(grant.id).unwrap_err().code,
            ErrorCode::LocalAccess
        );
        assert_eq!(
            service
                .consume_download_directory(grant.id)
                .unwrap_err()
                .code,
            ErrorCode::LocalAccess
        );

        let expired = service
            .insert(
                LocalGrantKind::DownloadDirectory,
                view.clone(),
                GrantValue::DownloadDirectory(directory.path().to_path_buf()),
            )
            .unwrap()
            .unwrap();
        service
            .grants
            .lock()
            .unwrap()
            .get_mut(&expired.id)
            .unwrap()
            .expires = Instant::now() - Duration::from_millis(1);
        assert_eq!(
            service
                .consume_download_directory(expired.id)
                .unwrap_err()
                .code,
            ErrorCode::LocalAccess
        );

        for _ in 0..GRANT_CAP {
            service
                .insert(
                    LocalGrantKind::DownloadDirectory,
                    view.clone(),
                    GrantValue::DownloadDirectory(directory.path().to_path_buf()),
                )
                .unwrap();
        }
        assert_eq!(
            service
                .insert(
                    LocalGrantKind::DownloadDirectory,
                    view,
                    GrantValue::DownloadDirectory(directory.path().to_path_buf())
                )
                .unwrap_err()
                .code,
            ErrorCode::LocalAccess
        );
        service.revoke_all();
        assert!(service.grants.lock().unwrap().is_empty());
    }
}
