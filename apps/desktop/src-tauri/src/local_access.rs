use nexus_model::{
    AppError, ErrorCode, HostId, HostSessionId, LocalGrantId, LocalGrantKind, LocalSelectionGrant,
    LocalSelectionItem, SftpSessionId,
};
use nexus_sftp::{LocalDirectory, LocalItem};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

const GRANT_LIFETIME: Duration = Duration::from_secs(10 * 60);
const GRANT_CAP: usize = 64;
const FILE_CAP: usize = 32;

enum GrantValue {
    Upload(Vec<LocalItem>),
    DownloadDirectory(LocalDirectory),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrantScope {
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub sftp_session_id: SftpSessionId,
}
struct StoredGrant {
    value: GrantValue,
    scope: GrantScope,
    expires: Instant,
}

#[derive(Default)]
pub struct LocalAccessService {
    grants: Mutex<HashMap<LocalGrantId, StoredGrant>>,
}

impl LocalAccessService {
    pub async fn choose_upload_files(
        &self,
        scope: GrantScope,
    ) -> Result<Option<LocalSelectionGrant>, AppError> {
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
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    local_error("A selected filename cannot be represented losslessly.")
                })?
                .to_owned();
            nexus_sftp::validate_child_name(&name)?;
            let item = nexus_sftp::open_local_source(path, name.clone())?;
            views.push(LocalSelectionItem {
                display_name: nexus_sftp::display_name(&name),
                size_bytes: Some(item.size.to_string()),
            });
            items.push(item);
        }
        self.insert(
            LocalGrantKind::UploadFiles,
            views,
            GrantValue::Upload(items),
            scope,
        )
    }

    pub async fn choose_download_directory(
        &self,
        scope: GrantScope,
    ) -> Result<Option<LocalSelectionGrant>, AppError> {
        let Some(handle) = rfd::AsyncFileDialog::new()
            .set_title("Choose download destination")
            .pick_folder()
            .await
        else {
            return Ok(None);
        };
        let path = handle.path().to_path_buf();
        let directory = nexus_sftp::open_local_directory(path.clone())?;
        self.insert(
            LocalGrantKind::DownloadDirectory,
            vec![LocalSelectionItem {
                display_name: path.display().to_string(),
                size_bytes: None,
            }],
            GrantValue::DownloadDirectory(directory),
            scope,
        )
    }

    pub fn consume_upload(
        &self,
        id: LocalGrantId,
        scope: GrantScope,
    ) -> Result<Vec<LocalItem>, AppError> {
        match self.consume(id, scope)? {
            GrantValue::Upload(items) => Ok(items),
            _ => Err(local_error("The local grant has the wrong scope.")),
        }
    }
    pub fn consume_download_directory(
        &self,
        id: LocalGrantId,
        scope: GrantScope,
    ) -> Result<LocalDirectory, AppError> {
        match self.consume(id, scope)? {
            GrantValue::DownloadDirectory(path) => Ok(path),
            _ => Err(local_error("The local grant has the wrong scope.")),
        }
    }
    pub fn revoke_all(&self) {
        if let Ok(mut grants) = self.grants.lock() {
            grants.clear();
        }
    }
    pub fn revoke_session(&self, host_id: HostId, host_session_id: HostSessionId) {
        if let Ok(mut grants) = self.grants.lock() {
            grants.retain(|_, grant| {
                grant.scope.host_id != host_id || grant.scope.host_session_id != host_session_id
            });
        }
    }
    pub fn revoke_host(&self, host_id: HostId) {
        if let Ok(mut grants) = self.grants.lock() {
            grants.retain(|_, grant| grant.scope.host_id != host_id);
        }
    }

    fn insert(
        &self,
        kind: LocalGrantKind,
        items: Vec<LocalSelectionItem>,
        value: GrantValue,
        scope: GrantScope,
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
                scope,
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
    fn consume(&self, id: LocalGrantId, scope: GrantScope) -> Result<GrantValue, AppError> {
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
        if grant.scope != scope {
            return Err(local_error(
                "The local selection grant belongs to a different SFTP session.",
            ));
        }
        Ok(grant.value)
    }
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
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let selected = nexus_sftp::open_local_directory(directory.path().to_path_buf()).unwrap();
        let scope = GrantScope {
            host_id: HostId::new(),
            host_session_id: HostSessionId::new(),
            sftp_session_id: SftpSessionId::new(),
        };
        let view = vec![LocalSelectionItem {
            display_name: "Destination".into(),
            size_bytes: None,
        }];
        let grant = service
            .insert(
                LocalGrantKind::DownloadDirectory,
                view.clone(),
                GrantValue::DownloadDirectory(selected.clone()),
                scope,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            service.consume_upload(grant.id, scope).unwrap_err().code,
            ErrorCode::LocalAccess
        );
        assert_eq!(
            service
                .consume_download_directory(grant.id, scope)
                .unwrap_err()
                .code,
            ErrorCode::LocalAccess
        );

        let expired = service
            .insert(
                LocalGrantKind::DownloadDirectory,
                view.clone(),
                GrantValue::DownloadDirectory(selected.clone()),
                scope,
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
                .consume_download_directory(expired.id, scope)
                .unwrap_err()
                .code,
            ErrorCode::LocalAccess
        );

        for _ in 0..GRANT_CAP {
            service
                .insert(
                    LocalGrantKind::DownloadDirectory,
                    view.clone(),
                    GrantValue::DownloadDirectory(selected.clone()),
                    scope,
                )
                .unwrap();
        }
        assert_eq!(
            service
                .insert(
                    LocalGrantKind::DownloadDirectory,
                    view,
                    GrantValue::DownloadDirectory(selected.clone()),
                    scope,
                )
                .unwrap_err()
                .code,
            ErrorCode::LocalAccess
        );
        service.revoke_all();
        assert!(service.grants.lock().unwrap().is_empty());
    }

    #[test]
    fn grants_are_bound_to_host_connection_and_sftp_session() {
        let service = LocalAccessService::default();
        let temporary = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let selected = nexus_sftp::open_local_directory(temporary.path().to_path_buf()).unwrap();
        let scope = GrantScope {
            host_id: HostId::new(),
            host_session_id: HostSessionId::new(),
            sftp_session_id: SftpSessionId::new(),
        };
        let grant = service
            .insert(
                LocalGrantKind::DownloadDirectory,
                vec![],
                GrantValue::DownloadDirectory(selected.clone()),
                scope,
            )
            .unwrap()
            .unwrap();
        let other_host = GrantScope {
            host_id: HostId::new(),
            ..scope
        };
        assert_eq!(
            service
                .consume_download_directory(grant.id, other_host)
                .unwrap_err()
                .code,
            ErrorCode::LocalAccess
        );

        let grant = service
            .insert(
                LocalGrantKind::DownloadDirectory,
                vec![],
                GrantValue::DownloadDirectory(selected),
                scope,
            )
            .unwrap()
            .unwrap();
        service.revoke_session(scope.host_id, scope.host_session_id);
        assert_eq!(
            service
                .consume_download_directory(grant.id, scope)
                .unwrap_err()
                .code,
            ErrorCode::LocalAccess
        );
    }
}
