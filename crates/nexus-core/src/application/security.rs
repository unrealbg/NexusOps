use super::*;

impl Application {
    /// Read local metadata atomically with respect to host edit/delete and trust acceptance.
    /// This does not reconcile sessions, access secrets, audit, or perform network I/O.
    pub async fn get_host_ssh_trust(&self, id: HostId) -> Result<SshEndpointTrust, AppError> {
        let _host_operation = self.remote_operations.lifecycle_guard(id).await?;
        let _mutation = self.mutation.lock().await;
        let host = self.repository.get(id)?.host;
        host.connection.validate().map_err(|_| {
            AppError::new(
                ErrorCode::Persistence,
                "The stored SSH endpoint configuration is invalid.",
            )
        })?;
        let endpoint_pin = self
            .known_hosts
            .fingerprint(&host.connection.hostname, host.connection.port)?;
        Ok(SshEndpointTrust {
            host_id: host.id,
            hostname: host.connection.hostname,
            port: host.connection.port,
            authentication: host.connection.authentication,
            endpoint_pin,
        })
    }
}
