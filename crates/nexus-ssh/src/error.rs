use nexus_model::{AppError, ErrorCode};
use std::io;

// Remote protocol text and key material must not escape via library error formatting.
pub(crate) enum HandlerError {
    Application(AppError),
    Transport(russh::Error),
}

impl std::fmt::Debug for HandlerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SshHandlerError")
    }
}

impl From<russh::Error> for HandlerError {
    fn from(error: russh::Error) -> Self {
        Self::Transport(error)
    }
}

impl HandlerError {
    pub(crate) fn into_app(self) -> AppError {
        match self {
            Self::Application(error) => error,
            Self::Transport(error) => transport_error("handshake", &error),
        }
    }
}

pub(crate) fn io_error(stage: &'static str, error: &io::Error) -> AppError {
    tracing::warn!(stage, io_kind = ?error.kind(), os_code = error.raw_os_error(), "SSH network failure");
    match error.kind() {
        io::ErrorKind::ConnectionRefused => AppError::new(
            ErrorCode::Refused,
            "The server refused the SSH connection. Check its address, port and SSH service.",
        ),
        io::ErrorKind::TimedOut => {
            AppError::new(ErrorCode::Timeout, "The SSH network connection timed out.")
        }
        _ => AppError::new(
            ErrorCode::Connection,
            "The SSH network connection failed or closed.",
        ),
    }
}

pub(crate) fn transport_error(stage: &'static str, error: &russh::Error) -> AppError {
    tracing::warn!(stage, error_kind = ?std::mem::discriminant(error), "SSH transport failure");
    match error {
        russh::Error::IO(error) => io_error(stage, error),
        russh::Error::ConnectionTimeout
        | russh::Error::KeepaliveTimeout
        | russh::Error::InactivityTimeout => {
            AppError::new(ErrorCode::Timeout, "The SSH server stopped responding.")
        }
        russh::Error::NotAuthenticated
        | russh::Error::UnsupportedAuthMethod
        | russh::Error::NoAuthMethod => AppError::new(
            ErrorCode::Authentication,
            "The server did not accept this SSH authentication method.",
        ),
        russh::Error::NoCommonAlgo { .. } => AppError::new(
            ErrorCode::Connection,
            "The server has no compatible secure SSH algorithm.",
        ),
        _ => AppError::new(
            ErrorCode::Connection,
            "The SSH connection failed or was closed by the server.",
        ),
    }
}
