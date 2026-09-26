use super::*;
use async_trait::async_trait;
use nexus_operations::ReadOnlyCommand;
use std::sync::atomic::{AtomicUsize, Ordering};

struct PartialSession {
    calls: AtomicUsize,
    cancellation: Option<CancellationToken>,
}

#[async_trait]
impl RemoteSession for PartialSession {
    async fn execute(
        &self,
        command: ReadOnlyCommand,
        _: CancellationToken,
    ) -> Result<String, AppError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
        }
        match command {
            ReadOnlyCommand::Hostname => Ok("test-host\n".into()),
            ReadOnlyCommand::OsRelease => Err(AppError::new(
                ErrorCode::Connection,
                "DO NOT INCLUDE remote output or secrets",
            )),
            ReadOnlyCommand::Kernel => Ok("6.8.0\n".into()),
            ReadOnlyCommand::Architecture => Ok("x86_64\n".into()),
            ReadOnlyCommand::Uptime => Ok("100.25 200.0\n".into()),
            ReadOnlyCommand::LoadAverage => Ok("0.12 0.15 0.13 1/100 999\n".into()),
            ReadOnlyCommand::Memory => Ok("MemTotal: 1000 kB\nMemAvailable: 250 kB\n".into()),
            ReadOnlyCommand::RootFilesystem => Ok("dev 1000 400 550 43% /\n".into()),
            ReadOnlyCommand::CpuStat => Ok("cpu 1 0 1 8 0 0 0 0\n".into()),
            ReadOnlyCommand::NetworkDevices => Ok("Inter-| Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n lo: 1 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0\n".into()),
            ReadOnlyCommand::SystemServices => panic!("service inventory is not a connection probe"),
        }
    }
    async fn disconnect(&self) -> Result<(), AppError> {
        panic!("discovery must not disconnect");
    }
    fn is_closed(&self) -> bool {
        false
    }
}

#[tokio::test]
async fn optional_failure_preserves_remaining_metrics_and_connection() {
    let session = PartialSession {
        calls: AtomicUsize::new(0),
        cancellation: None,
    };
    let result = discover(&session, CancellationToken::new())
        .await
        .expect("partial discovery");
    assert_eq!(session.calls.load(Ordering::Relaxed), 8);
    assert_eq!(result.hostname.as_deref(), Some("test-host"));
    assert_eq!(result.kernel.as_deref(), Some("6.8.0"));
    assert_eq!(result.memory_used_bytes, Some(768000));
    assert_eq!(result.root_used_bytes, Some(409600));
    assert_eq!(result.warnings.len(), 1);
    assert!(!result.warnings[0].message.contains("secrets"));
    assert_eq!(capabilities(&result).len(), 2);
}

#[tokio::test]
async fn cancellation_stops_additional_probes() {
    let cancellation = CancellationToken::new();
    let session = PartialSession {
        calls: AtomicUsize::new(0),
        cancellation: Some(cancellation.clone()),
    };
    let error = discover(&session, cancellation)
        .await
        .expect_err("cancelled discovery");
    assert_eq!(error.code, ErrorCode::Cancelled);
    assert_eq!(session.calls.load(Ordering::Relaxed), 1);
}
