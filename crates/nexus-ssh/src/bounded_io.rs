use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
};
use tokio_util::sync::CancellationToken;

/// russh owns its background session task. Wake pending socket operations on
/// cancellation so dropping a connect future or provider session closes that task.
pub(crate) struct CancellableStream {
    stream: TcpStream,
    cancelled: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl CancellableStream {
    pub(crate) fn new(stream: TcpStream, cancellation: CancellationToken) -> Self {
        Self {
            stream,
            cancelled: Box::pin(cancellation.cancelled_owned()),
        }
    }

    fn check_cancelled(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        if self.cancelled.as_mut().poll(cx).is_ready() {
            Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "SSH session cancelled",
            ))
        } else {
            Ok(())
        }
    }
}

impl AsyncRead for CancellableStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.check_cancelled(cx)?;
        Pin::new(&mut self.stream).poll_read(cx, buffer)
    }
}

impl AsyncWrite for CancellableStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.check_cancelled(cx)?;
        Pin::new(&mut self.stream).poll_write(cx, buffer)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.check_cancelled(cx)?;
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

use nexus_model::{AppError, ErrorCode};
use std::time::Duration;
use tokio::time::timeout;

pub(crate) async fn bounded<T>(
    cancellation: &CancellationToken,
    duration: Duration,
    stage: &'static str,
    future: impl Future<Output = Result<T, AppError>>,
) -> Result<T, AppError> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(cancelled()),
        result = timeout(duration, future) => result.unwrap_or_else(|_| {
            tracing::warn!(stage, timeout_seconds = duration.as_secs(), "SSH operation timed out");
            Err(AppError::new(ErrorCode::Timeout, format!("SSH {stage} timed out.")))
        }),
    }
}

pub(crate) fn cancelled() -> AppError {
    AppError::new(ErrorCode::Cancelled, "The SSH operation was cancelled.")
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::{io::AsyncReadExt, net::TcpListener};

    #[tokio::test]
    async fn cancellation_wakes_an_idle_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listen");
        let stream = TcpStream::connect(listener.local_addr().expect("address"))
            .await
            .expect("connect");
        let (_peer, _) = listener.accept().await.expect("accept");
        let cancellation = CancellationToken::new();
        let mut stream = CancellableStream::new(stream, cancellation.clone());
        let read = tokio::spawn(async move { stream.read_u8().await });
        cancellation.cancel();
        let error = tokio::time::timeout(Duration::from_secs(1), read)
            .await
            .expect("bounded")
            .expect("task")
            .expect_err("cancelled");
        assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
    }
}
