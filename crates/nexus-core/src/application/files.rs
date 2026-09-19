use super::*;
use nexus_sftp::{LocalDirectory, LocalItem, MutationSpec, SftpClient};
use std::sync::Arc;

impl Application {
    pub async fn open_sftp(&self, host_id: HostId) -> Result<SftpSessionInfo, AppError> {
        self.repository.get(host_id)?;
        let startup = self
            .sftp_startups
            .lock()
            .await
            .entry(host_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _startup_guard = startup.lock().await;
        let slot = self.slot(host_id).await;
        let (transport, connection_id, generation) = {
            let data = slot.data.lock().await;
            if data.view.state != ConnectionState::Connected {
                return Err(AppError::new(
                    ErrorCode::SftpUnavailable,
                    "Connect this host before opening Files.",
                ));
            }
            (
                data.transport.clone().ok_or_else(|| {
                    AppError::new(ErrorCode::Connection, "The SSH session is unavailable.")
                })?,
                data.connection_id.ok_or_else(|| {
                    AppError::new(
                        ErrorCode::Connection,
                        "The SSH session identity is unavailable.",
                    )
                })?,
                data.generation,
            )
        };
        if let Some(existing) = self.sftp_sessions.lock().await.get(&host_id).cloned()
            && existing.info().host_session_id == connection_id
        {
            return Ok(existing.info());
        }
        let client = transport.open_sftp(host_id, connection_id).await?;
        let data = slot.data.lock().await;
        if data.generation != generation
            || data.connection_id != Some(connection_id)
            || data.view.state != ConnectionState::Connected
        {
            drop(data);
            client.close().await;
            return Err(cancelled());
        }
        drop(data);
        let old = self
            .sftp_sessions
            .lock()
            .await
            .insert(host_id, client.clone());
        if let Some(old) = old {
            old.close().await;
        }
        Ok(client.info())
    }

    pub async fn list_remote_directory(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        sftp_session_id: SftpSessionId,
        path: String,
    ) -> Result<DirectoryListing, AppError> {
        self.owned_sftp(host_id, host_session_id, sftp_session_id)
            .await?
            .list(&path, CancellationToken::new())
            .await
    }

    pub async fn remote_properties(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        sftp_session_id: SftpSessionId,
        path: String,
    ) -> Result<RemoteEntry, AppError> {
        self.owned_sftp(host_id, host_session_id, sftp_session_id)
            .await?
            .stat(&path)
            .await
    }

    pub async fn plan_upload(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        sftp_session_id: SftpSessionId,
        sources: Vec<LocalItem>,
        remote_directory: String,
        policy: ConflictPolicy,
    ) -> Result<FileOperationPlan, AppError> {
        let client = self
            .owned_sftp(host_id, host_session_id, sftp_session_id)
            .await?;
        self.file_plans
            .plan_upload(client, sources, &remote_directory, policy)
            .await
    }

    pub async fn plan_download(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        sftp_session_id: SftpSessionId,
        remote_paths: Vec<String>,
        local_directory: LocalDirectory,
        policy: ConflictPolicy,
    ) -> Result<FileOperationPlan, AppError> {
        let client = self
            .owned_sftp(host_id, host_session_id, sftp_session_id)
            .await?;
        nexus_sftp::validate_local_directory_handle(&local_directory)?;
        self.file_plans
            .plan_download(client, remote_paths, local_directory, policy)
            .await
    }

    pub async fn plan_create_directory(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        sftp_session_id: SftpSessionId,
        parent: String,
        name: String,
    ) -> Result<FileOperationPlan, AppError> {
        nexus_sftp::validate_child_name(&name)?;
        let path = nexus_sftp::join_remote(&parent, &name)?;
        let client = self
            .owned_sftp(host_id, host_session_id, sftp_session_id)
            .await?;
        if client.identity(&path).await?.is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The remote destination already exists.",
            ));
        }
        self.file_plans
            .plan_mutation(client, MutationSpec::CreateDirectory { path })
    }

    pub async fn plan_rename(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        sftp_session_id: SftpSessionId,
        source: String,
        new_name: String,
    ) -> Result<FileOperationPlan, AppError> {
        nexus_sftp::validate_child_name(&new_name)?;
        let destination = nexus_sftp::join_remote(&nexus_sftp::parent_remote(&source)?, &new_name)?;
        let client = self
            .owned_sftp(host_id, host_session_id, sftp_session_id)
            .await?;
        let identity = client.identity(&source).await?.ok_or_else(|| {
            AppError::new(ErrorCode::NotFound, "The remote source no longer exists.")
        })?;
        if identity.kind != RemoteEntryKind::File
            || !client.info().extensions.iter().any(|extension| {
                extension.name == "hardlink@openssh.com" && extension.version == "1"
            })
        {
            return Err(AppError::new(
                ErrorCode::SftpUnavailable,
                "Safe no-clobber rename is available only for regular files when hardlink@openssh.com v1 is negotiated.",
            ));
        }
        if client.identity(&destination).await?.is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The remote destination already exists.",
            ));
        }
        self.file_plans.plan_mutation(
            client,
            MutationSpec::Rename {
                source,
                destination,
                expected_source: identity,
            },
        )
    }

    pub async fn plan_delete(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        sftp_session_id: SftpSessionId,
        path: String,
    ) -> Result<FileOperationPlan, AppError> {
        let client = self
            .owned_sftp(host_id, host_session_id, sftp_session_id)
            .await?;
        let entry = client.stat(&path).await?;
        if matches!(
            entry.kind,
            RemoteEntryKind::Other | RemoteEntryKind::Unknown
        ) {
            return Err(AppError::new(
                ErrorCode::FilePolicy,
                "Special or unknown entries cannot be deleted.",
            ));
        }
        let expected = client.identity(&path).await?.ok_or_else(|| {
            AppError::new(ErrorCode::NotFound, "The remote entry no longer exists.")
        })?;
        self.file_plans.plan_mutation(
            client,
            MutationSpec::Delete {
                path,
                kind: entry.kind,
                expected,
            },
        )
    }

    pub async fn execute_file_plan(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        sftp_session_id: SftpSessionId,
        plan_id: FilePlanId,
    ) -> Result<Vec<TransferJob>, AppError> {
        let startup = self
            .sftp_startups
            .lock()
            .await
            .entry(host_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _startup_guard = startup.lock().await;
        self.owned_sftp(host_id, host_session_id, sftp_session_id)
            .await?;
        let plan = self
            .file_plans
            .consume(plan_id, host_id, host_session_id, sftp_session_id)?;
        let audit_kind = match plan.view.kind {
            FileOperationKind::CreateDirectory => "file.create_directory",
            FileOperationKind::Rename => "file.rename",
            FileOperationKind::Delete => "file.delete",
            _ => "file.transfer.accepted",
        };
        let started = Instant::now();
        let result = self.transfers.execute(plan).await;
        self.record(
            host_id,
            audit_kind,
            if result.is_ok() {
                AuditOutcome::Success
            } else {
                AuditOutcome::Failed
            },
            started,
        )?;
        result
    }

    pub async fn discard_file_plan(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        sftp_session_id: SftpSessionId,
        plan_id: FilePlanId,
    ) -> Result<(), AppError> {
        self.file_plans
            .discard(plan_id, host_id, host_session_id, sftp_session_id)
    }

    pub fn list_transfers(&self, host_id: Option<HostId>) -> Vec<TransferJob> {
        self.transfers.list(host_id)
    }
    pub async fn cancel_transfer(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        sftp_session_id: SftpSessionId,
        job_id: TransferJobId,
    ) -> Result<(), AppError> {
        self.owned_sftp(host_id, host_session_id, sftp_session_id)
            .await?;
        self.transfers
            .cancel(job_id, host_id, host_session_id, sftp_session_id)
    }
    pub async fn plan_retry_transfer(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        sftp_session_id: SftpSessionId,
        job_id: TransferJobId,
    ) -> Result<FileOperationPlan, AppError> {
        self.owned_sftp(host_id, host_session_id, sftp_session_id)
            .await?;
        self.transfers
            .retry_plan(
                &self.file_plans,
                job_id,
                host_id,
                host_session_id,
                sftp_session_id,
            )
            .await
    }

    pub(crate) async fn close_sftp(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
    ) -> Result<(), AppError> {
        let startup = self
            .sftp_startups
            .lock()
            .await
            .entry(host_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _startup_guard = startup.lock().await;
        self.file_plans.revoke_session(host_id, host_session_id);
        self.transfers
            .quiesce_session(host_id, host_session_id)
            .await?;
        let session = {
            let mut sessions = self.sftp_sessions.lock().await;
            match sessions.get(&host_id) {
                Some(value) if value.info().host_session_id == host_session_id => {
                    sessions.remove(&host_id)
                }
                _ => None,
            }
        };
        if let Some(session) = session {
            session.close().await;
        }
        Ok(())
    }

    pub async fn validate_sftp_session(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        sftp_session_id: SftpSessionId,
    ) -> Result<(), AppError> {
        self.owned_sftp(host_id, host_session_id, sftp_session_id)
            .await
            .map(|_| ())
    }

    async fn owned_sftp(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        sftp_session_id: SftpSessionId,
    ) -> Result<Arc<dyn SftpClient>, AppError> {
        let slot = self.slot(host_id).await;
        let data = slot.data.lock().await;
        if data.view.state != ConnectionState::Connected
            || data.connection_id != Some(host_session_id)
        {
            return Err(AppError::new(
                ErrorCode::Connection,
                "This file operation belongs to an old SSH connection.",
            ));
        }
        drop(data);
        let client = self
            .sftp_sessions
            .lock()
            .await
            .get(&host_id)
            .cloned()
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::SftpUnavailable,
                    "Open the Files workspace again to start SFTP.",
                )
            })?;
        if client.info().id != sftp_session_id || client.info().host_session_id != host_session_id {
            return Err(AppError::new(
                ErrorCode::Policy,
                "This file operation belongs to an old SFTP session.",
            ));
        }
        Ok(client)
    }
}
