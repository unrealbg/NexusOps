use super::*;

impl Application {
    pub async fn list_terminals(&self, host_id: HostId) -> Result<Vec<TerminalSession>, AppError> {
        self.repository.get(host_id)?;
        Ok(self.terminals.list(host_id).await)
    }

    pub async fn open_terminal(
        &self,
        host_id: HostId,
        size: TerminalSize,
    ) -> Result<TerminalSession, AppError> {
        self.repository.get(host_id)?;
        let slot = self.slot(host_id).await;
        let (connection_id, transport) = {
            let data = slot.data.lock().await;
            if data.view.state != ConnectionState::Connected {
                return Err(terminal_unavailable());
            }
            let connection_id = data.connection_id.ok_or_else(terminal_unavailable)?;
            let transport = data.transport.clone().ok_or_else(terminal_unavailable)?;
            if transport.is_closed() {
                return Err(terminal_unavailable());
            }
            (connection_id, transport)
        };
        self.terminals
            .open(host_id, connection_id, transport.as_ref(), size)
            .await
    }

    pub async fn poll_terminal(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        terminal_id: TerminalSessionId,
    ) -> Result<TerminalOutputBatch, AppError> {
        self.repository.get(host_id)?;
        self.terminals
            .poll((host_id, host_session_id, terminal_id))
            .await
    }

    pub async fn write_terminal(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        terminal_id: TerminalSessionId,
        data_base64: &str,
    ) -> Result<(), AppError> {
        self.require_current_connection(host_id, host_session_id)
            .await?;
        self.terminals
            .write((host_id, host_session_id, terminal_id), data_base64)
            .await
    }

    pub async fn resize_terminal(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        terminal_id: TerminalSessionId,
        size: TerminalSize,
    ) -> Result<(), AppError> {
        self.require_current_connection(host_id, host_session_id)
            .await?;
        self.terminals
            .resize((host_id, host_session_id, terminal_id), size)
            .await
    }

    pub async fn rename_terminal(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        terminal_id: TerminalSessionId,
        label: &str,
    ) -> Result<TerminalSession, AppError> {
        self.repository.get(host_id)?;
        self.terminals
            .rename((host_id, host_session_id, terminal_id), label)
            .await
    }

    pub async fn close_terminal(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        terminal_id: TerminalSessionId,
    ) -> Result<(), AppError> {
        self.repository.get(host_id)?;
        self.terminals
            .close((host_id, host_session_id, terminal_id))
            .await
    }

    async fn require_current_connection(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
    ) -> Result<(), AppError> {
        self.repository.get(host_id)?;
        let slot = self.slot(host_id).await;
        let data = slot.data.lock().await;
        if data.view.state != ConnectionState::Connected
            || data.connection_id != Some(host_session_id)
            || data
                .transport
                .as_ref()
                .is_none_or(|transport| transport.is_closed())
        {
            return Err(terminal_unavailable());
        }
        Ok(())
    }
}

fn terminal_unavailable() -> AppError {
    AppError::new(
        ErrorCode::TerminalUnavailable,
        "The SSH session is unavailable. Reconnect and open a new terminal.",
    )
}
