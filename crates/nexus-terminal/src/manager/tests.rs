use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Notify, mpsc};

struct MockChannel {
    incoming: Mutex<mpsc::UnboundedReceiver<Result<TerminalRead, AppError>>>,
    writes: Mutex<Vec<Vec<u8>>>,
    sizes: Mutex<Vec<TerminalSize>>,
    closes: AtomicUsize,
}

impl MockChannel {
    fn new() -> (
        Arc<Self>,
        mpsc::UnboundedSender<Result<TerminalRead, AppError>>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                incoming: Mutex::new(rx),
                writes: Mutex::new(Vec::new()),
                sizes: Mutex::new(Vec::new()),
                closes: AtomicUsize::new(0),
            }),
            tx,
        )
    }
}

#[async_trait]
impl TerminalChannel for MockChannel {
    async fn read(&self) -> Result<TerminalRead, AppError> {
        self.incoming
            .lock()
            .await
            .recv()
            .await
            .unwrap_or(Ok(TerminalRead::Closed))
    }

    async fn write(&self, data: &[u8]) -> Result<(), AppError> {
        self.writes.lock().await.push(data.to_vec());
        Ok(())
    }

    async fn resize(&self, size: TerminalSize) -> Result<(), AppError> {
        self.sizes.lock().await.push(size);
        Ok(())
    }

    async fn close(&self) -> Result<(), AppError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct MockConnector {
    channel: Arc<MockChannel>,
    stall: bool,
}

#[async_trait]
impl TerminalConnector for MockConnector {
    async fn open_terminal(
        &self,
        _: TerminalSize,
        cancellation: CancellationToken,
    ) -> Result<Arc<dyn TerminalChannel>, AppError> {
        if self.stall {
            cancellation.cancelled().await;
            return Err(cancelled());
        }
        Ok(self.channel.clone())
    }
}

fn size(columns: u32, rows: u32) -> TerminalSize {
    TerminalSize {
        columns,
        rows,
        pixel_width: columns * 9,
        pixel_height: rows * 18,
    }
}

#[tokio::test]
async fn lifecycle_input_resize_and_duplicate_close_are_safe() {
    let manager = TerminalManager::new();
    let host = HostId::new();
    let connection = HostSessionId::new();
    let (channel, _incoming) = MockChannel::new();
    let connector = MockConnector {
        channel: channel.clone(),
        stall: false,
    };
    let terminal = manager
        .open(host, connection, &connector, size(80, 24))
        .await
        .expect("open");
    assert_eq!(terminal.state, TerminalState::Open);

    let ownership = (host, connection, terminal.id);
    manager
        .write(ownership, &STANDARD.encode(b"printf safe\r"))
        .await
        .expect("write");
    manager
        .resize(ownership, size(101, 37))
        .await
        .expect("resize");
    assert_eq!(channel.writes.lock().await.as_slice(), &[b"printf safe\r"]);
    assert_eq!(channel.sizes.lock().await.as_slice(), &[size(101, 37)]);

    manager.close(ownership).await.expect("close");
    manager.close(ownership).await.expect("duplicate close");
    assert_eq!(channel.closes.load(Ordering::SeqCst), 1);
    assert!(manager.list(host).await.is_empty());
}

#[tokio::test]
async fn ownership_checks_prevent_cross_host_or_connection_access() {
    let manager = TerminalManager::new();
    let host_a = HostId::new();
    let host_b = HostId::new();
    let connection_a = HostSessionId::new();
    let connection_b = HostSessionId::new();
    let (channel, _incoming) = MockChannel::new();
    let connector = MockConnector {
        channel,
        stall: false,
    };
    let terminal = manager
        .open(host_a, connection_a, &connector, size(80, 24))
        .await
        .expect("open");

    for ownership in [
        (host_b, connection_a, terminal.id),
        (host_a, connection_b, terminal.id),
    ] {
        assert_eq!(
            manager
                .write(ownership, &STANDARD.encode(b"blocked"))
                .await
                .expect_err("ownership mismatch")
                .code,
            ErrorCode::Policy
        );
    }
    assert!(manager.list(host_b).await.is_empty());
    assert_eq!(manager.list(host_a).await.len(), 1);
}

#[tokio::test]
async fn disconnect_cancels_creation_and_old_connection_cannot_be_reused() {
    let manager = TerminalManager::new();
    let host = HostId::new();
    let connection = HostSessionId::new();
    let (channel, _incoming) = MockChannel::new();
    let connector = MockConnector {
        channel,
        stall: true,
    };
    let open = manager.open(host, connection, &connector, size(80, 24));
    let disconnect = async {
        tokio::task::yield_now().await;
        manager.disconnect_connection(host, connection).await;
    };
    let (result, ()) = tokio::join!(open, disconnect);
    assert_eq!(
        result.expect_err("cancelled creation").code,
        ErrorCode::Cancelled
    );
    let sessions = manager.list(host).await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].state, TerminalState::Disconnected);
}

#[tokio::test]
async fn output_is_binary_safe_batched_and_late_output_is_ignored_after_disconnect() {
    let manager = TerminalManager::new();
    let host = HostId::new();
    let connection = HostSessionId::new();
    let (channel, incoming) = MockChannel::new();
    let connector = MockConnector {
        channel,
        stall: false,
    };
    let terminal = manager
        .open(host, connection, &connector, size(80, 24))
        .await
        .expect("open");
    let ownership = (host, connection, terminal.id);
    let text = "ASCII Здравей ✓".as_bytes();
    incoming
        .send(Ok(TerminalRead::Data(text[..8].to_vec())))
        .expect("send first split");
    incoming
        .send(Ok(TerminalRead::Data(text[8..].to_vec())))
        .expect("send second split");
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    let batch = manager.poll(ownership).await.expect("poll");
    let decoded: Vec<u8> = batch
        .chunks_base64
        .iter()
        .flat_map(|chunk| STANDARD.decode(chunk).expect("base64"))
        .collect();
    assert_eq!(decoded, text);

    manager.disconnect_connection(host, connection).await;
    let _ = incoming.send(Ok(TerminalRead::Data(b"late secret".to_vec())));
    tokio::task::yield_now().await;
    let batch = manager.poll(ownership).await.expect("poll ended terminal");
    assert_eq!(batch.session.state, TerminalState::Disconnected);
    assert!(batch.chunks_base64.is_empty());
    assert_eq!(
        manager
            .write(ownership, &STANDARD.encode(b"late input"))
            .await
            .expect_err("ended terminal")
            .code,
        ErrorCode::TerminalUnavailable
    );
}

#[tokio::test]
async fn output_queue_stays_bounded_when_consumer_falls_behind() {
    let manager = TerminalManager::new();
    let host = HostId::new();
    let connection = HostSessionId::new();
    let (channel, incoming) = MockChannel::new();
    let connector = MockConnector {
        channel,
        stall: false,
    };
    let terminal = manager
        .open(host, connection, &connector, size(80, 24))
        .await
        .expect("open");
    for _ in 0..(OUTPUT_QUEUE_CAPACITY + 20) {
        incoming
            .send(Ok(TerminalRead::Data(vec![b'x'; OUTPUT_CHUNK_BYTES])))
            .expect("producer");
    }
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    let entry = manager
        .inner
        .lock()
        .await
        .sessions
        .get(&terminal.id)
        .cloned()
        .expect("entry");
    assert!(entry.output.lock().await.len() <= OUTPUT_QUEUE_CAPACITY);
    let batch = manager
        .poll((host, connection, terminal.id))
        .await
        .expect("bounded poll");
    let bytes: usize = batch
        .chunks_base64
        .iter()
        .map(|chunk| STANDARD.decode(chunk).expect("base64").len())
        .sum();
    assert!(bytes <= OUTPUT_BATCH_BYTES);
}

#[tokio::test]
async fn remote_completion_drains_every_output_batch_before_signalling_completion() {
    let manager = TerminalManager::new();
    let host = HostId::new();
    let connection = HostSessionId::new();
    let (channel, incoming) = MockChannel::new();
    let connector = MockConnector {
        channel: channel.clone(),
        stall: false,
    };
    let terminal = manager
        .open(host, connection, &connector, size(80, 24))
        .await
        .expect("open");
    let ownership = (host, connection, terminal.id);
    let mut expected = vec![b'a'; OUTPUT_BATCH_BYTES + OUTPUT_CHUNK_BYTES];
    expected.extend_from_slice(b"--unique-final-marker--");

    incoming
        .send(Ok(TerminalRead::Data(expected.clone())))
        .expect("produce output");
    incoming
        .send(Ok(TerminalRead::Closed))
        .expect("complete remote stream");

    tokio::time::timeout(Duration::from_secs(1), async {
        while manager.list(host).await[0].state != TerminalState::Closed {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("producer terminates before first poll");

    let mut actual = Vec::new();
    let mut polls = 0;
    loop {
        let batch = manager.poll(ownership).await.expect("poll output");
        polls += 1;
        for chunk in batch.chunks_base64 {
            actual.extend(STANDARD.decode(chunk).expect("base64"));
        }
        if batch.output_drained {
            assert_eq!(batch.session.state, TerminalState::Closed);
            break;
        }
        assert!(polls < OUTPUT_QUEUE_CAPACITY, "polling must terminate");
    }

    assert!(polls > 1, "the regression must cross a batch boundary");
    assert_eq!(
        actual, expected,
        "all output arrives exactly once and in order"
    );
    assert!(actual.ends_with(b"--unique-final-marker--"));
    assert_eq!(channel.closes.load(Ordering::SeqCst), 1);
}

struct StallingConnector {
    started: Arc<Notify>,
    dropped: Arc<AtomicUsize>,
}

struct RejectingConnector;

#[async_trait]
impl TerminalConnector for RejectingConnector {
    async fn open_terminal(
        &self,
        _: TerminalSize,
        _: CancellationToken,
    ) -> Result<Arc<dyn TerminalChannel>, AppError> {
        Err(AppError::new(
            ErrorCode::TerminalStartup,
            "Terminal startup output exceeded its limit.",
        ))
    }
}

struct DropCounter(Arc<AtomicUsize>);

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl TerminalConnector for StallingConnector {
    async fn open_terminal(
        &self,
        _: TerminalSize,
        cancellation: CancellationToken,
    ) -> Result<Arc<dyn TerminalChannel>, AppError> {
        let _drop_counter = DropCounter(self.dropped.clone());
        self.started.notify_one();
        cancellation.cancelled().await;
        Err(cancelled())
    }
}

#[tokio::test]
async fn startup_timeout_is_classified_and_releases_capacity_repeatedly() {
    let manager = TerminalManager::new();
    let host = HostId::new();
    let connection = HostSessionId::new();
    let (channel, _) = MockChannel::new();
    let connector = MockConnector {
        channel,
        stall: true,
    };

    for _ in 0..(MAX_OPEN_TERMINALS_PER_HOST + 2) {
        let error = manager
            .open_with_timeout(
                host,
                connection,
                &connector,
                size(80, 24),
                Duration::from_millis(1),
            )
            .await
            .expect_err("startup must time out");
        assert_eq!(error.code, ErrorCode::TerminalStartup);
    }

    assert!(manager.list(host).await.is_empty());

    let (channel, _) = MockChannel::new();
    let replacement = MockConnector {
        channel,
        stall: false,
    };
    assert_eq!(
        manager
            .open(host, connection, &replacement, size(80, 24))
            .await
            .expect("failed startups do not consume the eight-session capacity")
            .state,
        TerminalState::Open
    );
}

#[tokio::test]
async fn startup_rejection_releases_capacity_repeatedly() {
    let manager = TerminalManager::new();
    let host = HostId::new();
    let connection = HostSessionId::new();
    for _ in 0..(MAX_OPEN_TERMINALS_PER_HOST + 2) {
        assert_eq!(
            manager
                .open(host, connection, &RejectingConnector, size(80, 24))
                .await
                .expect_err("startup rejection")
                .code,
            ErrorCode::TerminalStartup
        );
    }
    assert!(manager.list(host).await.is_empty());

    let (channel, _) = MockChannel::new();
    manager
        .open(
            host,
            connection,
            &MockConnector {
                channel,
                stall: false,
            },
            size(80, 24),
        )
        .await
        .expect("startup rejection does not consume active capacity");
}

#[tokio::test]
async fn dropping_open_after_insertion_cancels_startup_and_releases_capacity() {
    let manager = Arc::new(TerminalManager::new());
    let host = HostId::new();
    let connection = HostSessionId::new();
    let started = Arc::new(Notify::new());
    let dropped_count = Arc::new(AtomicUsize::new(0));
    let connector = Arc::new(StallingConnector {
        started: started.clone(),
        dropped: dropped_count.clone(),
    });
    let task = {
        let manager = manager.clone();
        let connector = connector.clone();
        tokio::spawn(async move {
            manager
                .open(host, connection, connector.as_ref(), size(80, 24))
                .await
        })
    };
    started.notified().await;
    task.abort();
    let _ = task.await;

    tokio::time::timeout(Duration::from_secs(1), async {
        while dropped_count.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the partially established connector future is dropped");
    assert!(manager.list(host).await.is_empty());

    let (channel, _) = MockChannel::new();
    let replacement = MockConnector {
        channel,
        stall: false,
    };
    manager
        .open(host, connection, &replacement, size(80, 24))
        .await
        .expect("caller cancellation releases capacity");
}
