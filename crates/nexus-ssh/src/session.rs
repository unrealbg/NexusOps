use crate::{
    bounded_io::{bounded, cancelled},
    error::transport_error,
    handler::Client,
};
use async_trait::async_trait;
use nexus_model::{AppError, ErrorCode};
use nexus_operations::{ReadOnlyCommand, RemoteSession};
use nexus_terminal::{TerminalChannel, TerminalConnector, TerminalRead};
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Disconnect, client};
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{sync::Mutex, time::timeout};
use tokio_util::sync::{CancellationToken, DropGuard};
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
pub(crate) const OUTPUT_LIMIT: usize = 64 * 1024;
pub struct SshSession {
    pub(crate) handle: Mutex<client::Handle<Client>>,
    pub(crate) lifetime: CancellationToken,
    pub(crate) _guard: DropGuard,
    pub(crate) closed: Arc<AtomicBool>,
}

struct RusshTerminalChannel {
    reader: Mutex<TerminalReader>,
    writer: Arc<ChannelWriteHalf<client::Msg>>,
    transport_lifetime: CancellationToken,
    closed: AtomicBool,
}

struct TerminalReader {
    channel: ChannelReadHalf,
    pending: VecDeque<Vec<u8>>,
}

#[async_trait]
impl TerminalConnector for SshSession {
    async fn open_terminal(
        &self,
        size: nexus_model::TerminalSize,
        cancellation: CancellationToken,
    ) -> Result<Arc<dyn TerminalChannel>, AppError> {
        size.validate()?;
        if self.is_closed() {
            return Err(terminal_unavailable());
        }
        let mut channel = {
            let handle = self.handle.lock().await;
            handle.channel_open_session().await.map_err(|_| {
                terminal_failure(
                    ErrorCode::TerminalChannel,
                    "The SSH terminal channel could not be opened.",
                    "terminal_channel_open",
                )
            })?
        };
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(cancelled()),
            _ = self.lifetime.cancelled() => return Err(terminal_unavailable()),
            result = channel.request_pty(
                true,
                "xterm-256color",
                size.columns,
                size.rows,
                size.pixel_width,
                size.pixel_height,
                &[],
            ) => result.map_err(|_| terminal_failure(
                ErrorCode::TerminalPty,
                "The server rejected the terminal PTY allocation.",
                "terminal_pty_request",
            ))?,
        }
        let mut pending = wait_for_request_reply(
            &mut channel,
            &cancellation,
            &self.lifetime,
            ErrorCode::TerminalPty,
            "The server rejected the terminal PTY allocation.",
        )
        .await?;
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(cancelled()),
            _ = self.lifetime.cancelled() => return Err(terminal_unavailable()),
            result = channel.request_shell(true) => result.map_err(|_| terminal_failure(
                ErrorCode::TerminalShell,
                "The server rejected the interactive shell request.",
                "terminal_shell_request",
            ))?,
        }
        pending.extend(
            wait_for_request_reply(
                &mut channel,
                &cancellation,
                &self.lifetime,
                ErrorCode::TerminalShell,
                "The server rejected the interactive shell request.",
            )
            .await?,
        );
        let (reader, writer) = channel.split();
        Ok(Arc::new(RusshTerminalChannel {
            reader: Mutex::new(TerminalReader {
                channel: reader,
                pending,
            }),
            writer: Arc::new(writer),
            transport_lifetime: self.lifetime.clone(),
            closed: AtomicBool::new(false),
        }))
    }
}

#[async_trait]
impl TerminalChannel for RusshTerminalChannel {
    async fn read(&self) -> Result<TerminalRead, AppError> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(TerminalRead::Closed);
        }
        let mut reader = self.reader.lock().await;
        if let Some(data) = reader.pending.pop_front() {
            return Ok(TerminalRead::Data(data));
        }
        loop {
            let message = tokio::select! {
                biased;
                _ = self.transport_lifetime.cancelled() => return Err(terminal_unavailable()),
                message = reader.channel.wait() => message,
            };
            match message {
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                    return Ok(TerminalRead::Data(data.to_vec()));
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    return Ok(TerminalRead::Exit(exit_status));
                }
                Some(ChannelMsg::ExitSignal { .. })
                | Some(ChannelMsg::Eof)
                | Some(ChannelMsg::Close)
                | None => return Ok(TerminalRead::Closed),
                Some(ChannelMsg::Failure) => {
                    return Err(terminal_failure(
                        ErrorCode::TerminalChannel,
                        "The remote terminal channel failed.",
                        "terminal_read",
                    ));
                }
                Some(_) => {}
            }
        }
    }

    async fn write(&self, data: &[u8]) -> Result<(), AppError> {
        if self.closed.load(Ordering::Acquire) || self.transport_lifetime.is_cancelled() {
            return Err(terminal_unavailable());
        }
        self.writer.data_bytes(data.to_vec()).await.map_err(|_| {
            terminal_failure(
                ErrorCode::TerminalStream,
                "Terminal input could not be delivered.",
                "terminal_write",
            )
        })
    }

    async fn resize(&self, size: nexus_model::TerminalSize) -> Result<(), AppError> {
        size.validate()?;
        if self.closed.load(Ordering::Acquire) || self.transport_lifetime.is_cancelled() {
            return Err(terminal_unavailable());
        }
        self.writer
            .window_change(size.columns, size.rows, size.pixel_width, size.pixel_height)
            .await
            .map_err(|_| {
                terminal_failure(
                    ErrorCode::TerminalResize,
                    "The remote terminal could not be resized.",
                    "terminal_resize",
                )
            })
    }

    async fn close(&self) -> Result<(), AppError> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let _ = self.writer.eof().await;
        self.writer.close().await.map_err(|_| {
            terminal_failure(
                ErrorCode::TerminalChannel,
                "The remote terminal channel could not be closed cleanly.",
                "terminal_close",
            )
        })
    }
}

async fn wait_for_request_reply(
    channel: &mut russh::Channel<client::Msg>,
    cancellation: &CancellationToken,
    transport_lifetime: &CancellationToken,
    code: ErrorCode,
    message: &'static str,
) -> Result<VecDeque<Vec<u8>>, AppError> {
    let reply = async {
        let mut pending = VecDeque::new();
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => return Ok(pending),
                Some(ChannelMsg::Failure) | Some(ChannelMsg::Close) | None => {
                    return Err(terminal_failure(code, message, "terminal_request_reply"));
                }
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                    pending.push_back(data.to_vec())
                }
                Some(_) => {}
            }
        }
    };
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(cancelled()),
        _ = transport_lifetime.cancelled() => Err(terminal_unavailable()),
        result = timeout(COMMAND_TIMEOUT, reply) => result.map_err(|_| AppError::new(
            ErrorCode::TerminalStartup,
            "The server did not confirm terminal startup in time.",
        ))?,
    }
}

fn terminal_unavailable() -> AppError {
    AppError::new(
        ErrorCode::TerminalUnavailable,
        "The SSH session is unavailable. Reconnect and open a new terminal.",
    )
}

fn terminal_failure(code: ErrorCode, message: &'static str, stage: &'static str) -> AppError {
    tracing::debug!(stage = stage, "SSH terminal operation failed");
    AppError::new(code, message)
}

#[async_trait]
impl RemoteSession for SshSession {
    async fn execute(
        &self,
        command: ReadOnlyCommand,
        cancellation: CancellationToken,
    ) -> Result<String, AppError> {
        if self.is_closed() {
            return Err(AppError::new(
                ErrorCode::Connection,
                "This SSH session is disconnected.",
            ));
        }
        let operation = cancellation.child_token();
        let run = async {
            let channel = {
                let handle = self.handle.lock().await;
                handle
                    .channel_open_session()
                    .await
                    .map_err(|error| transport_error("channel_open", &error))?
            };
            let (mut reader, writer) = channel.split();
            let mut guard = ChannelGuard {
                writer: Arc::new(writer),
                closed: false,
            };
            let result = bounded(&operation, COMMAND_TIMEOUT, "command", async {
                guard
                    .writer
                    .exec(true, command.command())
                    .await
                    .map_err(|error| transport_error("command_start", &error))?;
                collect_output(&mut reader).await
            })
            .await;
            guard.close().await;
            result
        };
        tokio::select! {
            biased;
            _ = self.lifetime.cancelled() => Err(cancelled()),
            result = bounded(&cancellation, COMMAND_TIMEOUT + CLOSE_TIMEOUT, "channel", run) => result,
        }
    }

    async fn disconnect(&self) -> Result<(), AppError> {
        if self.is_closed() {
            self.lifetime.cancel();
            return Ok(());
        }
        self.closed.store(true, Ordering::Release);
        let result = timeout(CLOSE_TIMEOUT, async {
            self.handle
                .lock()
                .await
                .disconnect(Disconnect::ByApplication, "", "")
                .await
        })
        .await;
        // Always tear down I/O, even if the peer ignores protocol disconnect.
        self.lifetime.cancel();
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(transport_error("disconnect", &error)),
            Err(_) => Err(AppError::new(
                ErrorCode::Timeout,
                "SSH disconnect timed out; the local session was closed.",
            )),
        }
    }

    fn is_closed(&self) -> bool {
        self.lifetime.is_cancelled() || self.closed.load(Ordering::Acquire)
    }
}

/// Closing on drop also covers a caller (such as the operation engine) dropping
/// its execution future. Cleanup is independent of the cancelled operation token.
struct ChannelGuard {
    writer: Arc<ChannelWriteHalf<client::Msg>>,
    closed: bool,
}

impl ChannelGuard {
    async fn close(&mut self) {
        close_channel(&self.writer).await;
        self.closed = true;
    }
}

impl Drop for ChannelGuard {
    fn drop(&mut self) {
        if !self.closed {
            let writer = self.writer.clone();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move { close_channel(&writer).await });
            }
        }
    }
}

async fn close_channel(writer: &ChannelWriteHalf<client::Msg>) {
    match timeout(CLOSE_TIMEOUT, writer.close()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let _ = transport_error("channel_close", &error);
        }
        Err(_) => tracing::debug!(stage = "channel_close", "SSH channel cleanup timed out"),
    }
}

async fn collect_output(channel: &mut ChannelReadHalf) -> Result<String, AppError> {
    let mut output = Vec::new();
    let mut total = 0usize;
    let mut exit_status = None;
    while let Some(message) = channel.wait().await {
        match message {
            ChannelMsg::Data { data } => {
                count_output(&mut total, data.len())?;
                output.extend_from_slice(&data);
            }
            ChannelMsg::ExtendedData { data, .. } => {
                // stderr counts toward the limit but is never exposed or logged.
                count_output(&mut total, data.len())?;
            }
            ChannelMsg::ExitStatus {
                exit_status: status,
            } => exit_status = Some(status),
            ChannelMsg::Failure => {
                return Err(AppError::new(
                    ErrorCode::Discovery,
                    "The server rejected a read-only discovery command.",
                ));
            }
            ChannelMsg::ExitSignal { .. } => {
                return Err(AppError::new(
                    ErrorCode::Discovery,
                    "A read-only discovery command was interrupted on the server.",
                ));
            }
            ChannelMsg::Close => break,
            _ => {}
        }
    }
    if exit_status != Some(0) {
        tracing::debug!(
            stage = "command_exit",
            exit_status,
            "Read-only probe failed"
        );
        return Err(AppError::new(
            ErrorCode::Discovery,
            "A read-only discovery command did not complete successfully.",
        ));
    }
    String::from_utf8(output).map_err(|_| {
        AppError::new(
            ErrorCode::Discovery,
            "The server returned invalid text for a discovery command.",
        )
    })
}

fn count_output(total: &mut usize, length: usize) -> Result<(), AppError> {
    *total = total.saturating_add(length);
    if *total > OUTPUT_LIMIT {
        Err(AppError::new(
            ErrorCode::Discovery,
            "A discovery command exceeded the 64 KiB output limit.",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_stdout_and_stderr_is_bounded() {
        let mut total = 0;
        count_output(&mut total, OUTPUT_LIMIT).expect("exact limit");
        assert_eq!(
            count_output(&mut total, 1).expect_err("over limit").code,
            ErrorCode::Discovery
        );
    }

    #[tokio::test]
    async fn cancellation_prevents_starting_work() {
        let token = CancellationToken::new();
        token.cancel();
        let result = bounded(&token, Duration::from_secs(1), "test", async {
            panic!("cancelled work must never start");
            #[allow(unreachable_code)]
            Ok(())
        })
        .await;
        assert_eq!(result.expect_err("cancelled").code, ErrorCode::Cancelled);
    }

    #[tokio::test]
    async fn stalled_work_times_out() {
        let result = bounded(
            &CancellationToken::new(),
            Duration::from_millis(10),
            "test",
            std::future::pending::<Result<(), AppError>>(),
        )
        .await;
        assert_eq!(result.expect_err("timeout").code, ErrorCode::Timeout);
    }
}
