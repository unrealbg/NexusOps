use super::*;

fn stale_session() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "The network request does not belong to the active host session.",
    )
}

fn busy() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "A network inventory request is already in progress or the network limit is reached.",
    )
}

impl Application {
    /// A session-bound, non-queuing observation. No network data is retained.
    pub async fn list_host_network(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
    ) -> Result<NetworkSnapshot, AppError> {
        self.with_network_admission(
            host_id,
            host_session_id,
            |transport, cancellation| async move {
                nexus_discovery::observe_network(
                    transport.as_ref(),
                    cancellation,
                    host_id,
                    host_session_id,
                )
                .await
            },
        )
        .await
    }

    /// Both required family reads share the existing Network admission permits.
    pub async fn list_host_routes(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
    ) -> Result<NetworkRouteSnapshot, AppError> {
        self.with_network_admission(
            host_id,
            host_session_id,
            |transport, cancellation| async move {
                nexus_discovery::observe_routes(
                    transport.as_ref(),
                    cancellation,
                    host_id,
                    host_session_id,
                )
                .await
            },
        )
        .await
    }

    /// One bounded socket read shares admission with interfaces and routes.
    pub async fn list_host_listening_ports(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
    ) -> Result<ListeningPortSnapshot, AppError> {
        self.with_network_admission(
            host_id,
            host_session_id,
            |transport, cancellation| async move {
                nexus_discovery::observe_listening_ports(
                    transport.as_ref(),
                    cancellation,
                    host_id,
                    host_session_id,
                )
                .await
            },
        )
        .await
    }

    /// One bounded `/etc/resolv.conf` read shares admission with all Network observations.
    pub async fn list_host_dns_configuration(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
    ) -> Result<DnsConfigurationSnapshot, AppError> {
        self.with_network_admission(
            host_id,
            host_session_id,
            |transport, cancellation| async move {
                nexus_discovery::observe_dns_configuration(
                    transport.as_ref(),
                    cancellation,
                    host_id,
                    host_session_id,
                )
                .await
            },
        )
        .await
    }

    async fn with_network_admission<T, F, Fut>(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        observe: F,
    ) -> Result<T, AppError>
    where
        F: FnOnce(Arc<dyn crate::ConnectedTransport>, CancellationToken) -> Fut,
        Fut: std::future::Future<Output = Result<T, AppError>>,
    {
        let mutation = self.mutation.try_lock().map_err(|_| busy())?;
        self.repository.get(host_id)?;
        let slot = self.slot(host_id).await;
        let (transport, cancellation, generation) = {
            let data = slot.data.lock().await;
            if data.view.state != ConnectionState::Connected
                || data.connection_id != Some(host_session_id)
                || data.view.host_session_id != Some(host_session_id)
            {
                return Err(stale_session());
            }
            (
                data.transport.clone().ok_or_else(stale_session)?,
                data.cancel.clone(),
                data.generation,
            )
        };
        let gate = {
            let mut gates = self.network_gates.lock().await;
            gates
                .entry(host_id)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _host_permit = gate.try_lock().map_err(|_| busy())?;
        let _global_permit = self.network_limit.try_acquire().map_err(|_| busy())?;
        drop(mutation);
        let result = observe(transport, cancellation.clone()).await;
        let data = slot.data.lock().await;
        if cancellation.is_cancelled()
            || data.generation != generation
            || data.view.state != ConnectionState::Connected
            || data.connection_id != Some(host_session_id)
            || data.view.host_session_id != Some(host_session_id)
        {
            return Err(cancelled());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nexus_operations::{ReadOnlyCommand, RemoteSession};
    use nexus_remote_operations::{
        ConsumedAuthority, MutationTransport, MutationTransportOutcome, SystemdReload,
        SystemdResetFailed, SystemdStart, SystemdTryRestart,
    };
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    struct Transport {
        stall: Option<ReadOnlyCommand>,
        calls: StdMutex<Vec<ReadOnlyCommand>>,
        fail: std::sync::atomic::AtomicBool,
        entered: Semaphore,
        release: Semaphore,
    }
    #[async_trait]
    impl RemoteSession for Transport {
        async fn execute(
            &self,
            command: ReadOnlyCommand,
            _: CancellationToken,
        ) -> Result<String, AppError> {
            self.calls.lock().unwrap().push(command);
            if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(AppError::new(
                    ErrorCode::Connection,
                    "Synthetic read failure",
                ));
            }
            if self.stall == Some(command) {
                self.entered.add_permits(1);
                self.release.acquire().await.unwrap().forget();
            }
            Ok(match command {
                ReadOnlyCommand::ListeningPorts => "tcp LISTEN 0 128 192.0.2.10:8080 *:*",
                ReadOnlyCommand::DnsConfiguration => {
                    "nameserver 127.0.0.53\nsearch example.test\noptions edns0\n"
                }
                ReadOnlyCommand::NetworkRoutesV4 => {
                    r#"[{"dst":"default","gateway":"192.0.2.1","dev":"eth0"}]"#
                }
                ReadOnlyCommand::NetworkRoutesV6 => r#"[{"dst":"2001:db8::/64","dev":"eth0"}]"#,
                ReadOnlyCommand::NetworkAddresses => "[]",
                _ => panic!("Only explicit synthetic Network observations are permitted"),
            }
            .into())
        }
        async fn disconnect(&self) -> Result<(), AppError> {
            Ok(())
        }
        fn is_closed(&self) -> bool {
            false
        }
    }
    macro_rules! no_mutation {
        ($kind:ty) => {
            #[async_trait]
            impl MutationTransport<$kind> for Transport {
                async fn dispatch(
                    &self,
                    _: &ConsumedAuthority<$kind>,
                    _: CancellationToken,
                ) -> MutationTransportOutcome {
                    panic!("No mutation is permitted in route tests")
                }
            }
        };
    }
    no_mutation!(SystemdResetFailed);
    no_mutation!(SystemdTryRestart);
    no_mutation!(SystemdReload);
    no_mutation!(SystemdStart);
    #[async_trait]
    impl nexus_terminal::TerminalConnector for Transport {
        async fn open_terminal(
            &self,
            _: TerminalSize,
            _: CancellationToken,
        ) -> Result<Arc<dyn nexus_terminal::TerminalChannel>, AppError> {
            panic!("No terminal")
        }
    }
    #[async_trait]
    impl nexus_sftp::SftpConnector for Transport {
        async fn open_sftp(
            &self,
            _: HostId,
            _: HostSessionId,
        ) -> Result<Arc<dyn nexus_sftp::SftpClient>, AppError> {
            panic!("No SFTP")
        }
    }

    fn setup(
        stall: Option<ReadOnlyCommand>,
    ) -> (tempfile::TempDir, Arc<Application>, Arc<Transport>) {
        let dir = tempfile::tempdir().unwrap();
        // An empty disposable profile only. No connect/credential/keychain API is
        // called; synthetic connected transports are installed directly below.
        let app = Application::open(dir.path()).unwrap();
        let transport = transport(stall);
        (dir, Arc::new(app), transport)
    }
    fn transport(stall: Option<ReadOnlyCommand>) -> Arc<Transport> {
        Arc::new(Transport {
            stall,
            calls: StdMutex::new(vec![]),
            fail: std::sync::atomic::AtomicBool::new(false),
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
        })
    }
    async fn install(app: &Application, transport: Arc<Transport>) -> (HostId, HostSessionId) {
        let host = HostId::new();
        let session = HostSessionId::new();
        app.repository
            .save(&StoredHost {
                credential_id: HostId::new(),
                host: Host {
                    id: host,
                    display_name: "Synthetic".into(),
                    connection: HostConnectionConfig {
                        hostname: "192.0.2.10".into(),
                        port: 22,
                        username: "fixture".into(),
                        authentication: AuthenticationMethod::Password,
                    },
                },
            })
            .unwrap();
        let slot = app.slot(host).await;
        let mut data = slot.data.lock().await;
        data.view.state = ConnectionState::Connected;
        data.view.host_session_id = Some(session);
        data.connection_id = Some(session);
        data.transport = Some(transport);
        (host, session)
    }
    async fn entered(transport: &Transport) {
        tokio::time::timeout(Duration::from_secs(2), transport.entered.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
    }

    #[tokio::test]
    async fn routes_require_exact_session_and_complete_owned_observation_without_persistence() {
        let (dir, app, transport) = setup(None);
        let (host, session) = install(&app, transport.clone()).await;
        assert_eq!(
            app.list_host_routes(host, HostSessionId::new())
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert!(transport.calls.lock().unwrap().is_empty());
        let audit = std::fs::read(dir.path().join("audit.jsonl")).unwrap_or_default();
        let metadata = app.list_hosts().unwrap();
        let snapshot = app.list_host_routes(host, session).await.unwrap();
        assert_eq!(snapshot.host_id, host);
        assert_eq!(snapshot.host_session_id, session);
        assert_eq!(snapshot.entries.len(), 2);
        assert_eq!(
            *transport.calls.lock().unwrap(),
            vec![
                ReadOnlyCommand::NetworkRoutesV4,
                ReadOnlyCommand::NetworkRoutesV6
            ]
        );
        assert_eq!(app.network_limit.available_permits(), 4);
        assert_eq!(app.list_hosts().unwrap(), metadata);
        assert_eq!(
            std::fs::read(dir.path().join("audit.jsonl")).unwrap_or_default(),
            audit
        );
    }

    #[tokio::test]
    async fn both_family_reads_hold_the_same_network_admission_and_release_on_drop() {
        for stall in [
            ReadOnlyCommand::NetworkRoutesV4,
            ReadOnlyCommand::NetworkRoutesV6,
        ] {
            let (_dir, app, transport) = setup(Some(stall));
            let (host, session) = install(&app, transport.clone()).await;
            let request_app = app.clone();
            let request =
                tokio::spawn(async move { request_app.list_host_routes(host, session).await });
            entered(&transport).await;
            assert_eq!(app.network_limit.available_permits(), 3);
            assert_eq!(
                app.list_host_network(host, session).await.unwrap_err().code,
                ErrorCode::Conflict
            );
            assert_eq!(
                app.list_host_routes(host, session).await.unwrap_err().code,
                ErrorCode::Conflict
            );
            request.abort();
            let _ = request.await;
            assert_eq!(app.network_limit.available_permits(), 4);
            assert!(app.list_host_network(host, session).await.is_ok());
        }
        let (_dir, app, transport) = setup(Some(ReadOnlyCommand::NetworkAddresses));
        let (host, session) = install(&app, transport.clone()).await;
        let request_app = app.clone();
        let request =
            tokio::spawn(async move { request_app.list_host_network(host, session).await });
        entered(&transport).await;
        assert_eq!(
            app.list_host_routes(host, session).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        request.abort();
        let _ = request.await;
        assert!(app.list_host_routes(host, session).await.is_ok());
    }

    #[tokio::test]
    async fn post_io_generation_session_and_state_changes_reject_the_entire_snapshot() {
        for change in 0..3 {
            let (_dir, app, transport) = setup(Some(ReadOnlyCommand::NetworkRoutesV6));
            let (host, session) = install(&app, transport.clone()).await;
            let request_app = app.clone();
            let request =
                tokio::spawn(async move { request_app.list_host_routes(host, session).await });
            entered(&transport).await;
            let slot = app.slot(host).await;
            {
                let mut data = slot.data.lock().await;
                match change {
                    0 => data.generation += 1,
                    1 => {
                        data.connection_id = Some(HostSessionId::new());
                    }
                    _ => data.view.state = ConnectionState::Disconnected,
                }
            }
            transport.release.add_permits(1);
            assert_eq!(
                request.await.unwrap().unwrap_err().code,
                ErrorCode::Cancelled
            );
            assert_eq!(app.network_limit.available_permits(), 4);
        }
    }

    #[tokio::test]
    async fn cancellation_during_second_family_never_publishes_partial_routes() {
        let (_dir, app, transport) = setup(Some(ReadOnlyCommand::NetworkRoutesV6));
        let (host, session) = install(&app, transport.clone()).await;
        let request_app = app.clone();
        let request =
            tokio::spawn(async move { request_app.list_host_routes(host, session).await });
        entered(&transport).await;
        app.slot(host).await.data.lock().await.cancel.cancel();
        assert_eq!(
            request.await.unwrap().unwrap_err().code,
            ErrorCode::Cancelled
        );
        assert_eq!(app.network_limit.available_permits(), 4);
    }

    #[tokio::test]
    async fn routes_share_four_global_slots_with_interface_reads() {
        let (_dir, app, transport) = setup(Some(ReadOnlyCommand::NetworkRoutesV6));
        let mut requests = vec![];
        for _ in 0..4 {
            let (host, session) = install(&app, transport.clone()).await;
            let request_app = app.clone();
            requests.push(tokio::spawn(async move {
                request_app.list_host_routes(host, session).await
            }));
            entered(&transport).await;
        }
        let (host, session) = install(&app, transport.clone()).await;
        assert_eq!(
            app.list_host_routes(host, session).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(
            app.list_host_network(host, session).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        transport.release.add_permits(4);
        for request in requests {
            assert!(request.await.unwrap().is_ok());
        }
        assert_eq!(app.network_limit.available_permits(), 4);
    }
    #[tokio::test]
    async fn listening_ports_require_exact_owner_and_do_not_persist_or_use_service_admission() {
        let (dir, app, transport) = setup(None);
        let (host, owner) = install(&app, transport.clone()).await;
        assert_eq!(
            app.list_host_listening_ports(host, HostSessionId::new())
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert!(transport.calls.lock().unwrap().is_empty());
        let audit = std::fs::read(dir.path().join("audit.jsonl")).unwrap_or_default();
        let hosts = app.list_hosts().unwrap();
        let _services = app.service_limit.acquire_many(4).await.unwrap();
        let snapshot = app.list_host_listening_ports(host, owner).await.unwrap();
        assert_eq!(snapshot.host_id, host);
        assert_eq!(snapshot.host_session_id, owner);
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(
            *transport.calls.lock().unwrap(),
            vec![ReadOnlyCommand::ListeningPorts]
        );
        assert_eq!(app.list_hosts().unwrap(), hosts);
        assert_eq!(
            std::fs::read(dir.path().join("audit.jsonl")).unwrap_or_default(),
            audit
        );
        assert_eq!(app.network_limit.available_permits(), 4);
    }
    #[tokio::test]
    async fn all_four_network_observations_share_host_admission_and_release_dropped_requests() {
        for command in [
            ReadOnlyCommand::NetworkAddresses,
            ReadOnlyCommand::NetworkRoutesV4,
            ReadOnlyCommand::ListeningPorts,
            ReadOnlyCommand::DnsConfiguration,
        ] {
            let (_dir, app, transport) = setup(Some(command));
            let (host, owner) = install(&app, transport.clone()).await;
            let request_app = app.clone();
            let request = tokio::spawn(async move {
                match command {
                    ReadOnlyCommand::NetworkAddresses => {
                        request_app.list_host_network(host, owner).await.map(|_| ())
                    }
                    ReadOnlyCommand::NetworkRoutesV4 => {
                        request_app.list_host_routes(host, owner).await.map(|_| ())
                    }
                    ReadOnlyCommand::ListeningPorts => request_app
                        .list_host_listening_ports(host, owner)
                        .await
                        .map(|_| ()),
                    ReadOnlyCommand::DnsConfiguration => request_app
                        .list_host_dns_configuration(host, owner)
                        .await
                        .map(|_| ()),
                    _ => unreachable!(),
                }
            });
            entered(&transport).await;
            assert_eq!(
                app.list_host_network(host, owner).await.unwrap_err().code,
                ErrorCode::Conflict
            );
            assert_eq!(
                app.list_host_routes(host, owner).await.unwrap_err().code,
                ErrorCode::Conflict
            );
            assert_eq!(
                app.list_host_listening_ports(host, owner)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::Conflict
            );
            assert_eq!(
                app.list_host_dns_configuration(host, owner)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::Conflict
            );
            request.abort();
            let _ = request.await;
            assert_eq!(app.network_limit.available_permits(), 4);
            transport.release.add_permits(1);
            assert!(app.list_host_listening_ports(host, owner).await.is_ok());
        }
    }

    #[tokio::test]
    async fn dns_requires_exact_owner_does_not_persist_and_ignores_service_admission() {
        let (dir, app, transport) = setup(None);
        let (host, owner) = install(&app, transport.clone()).await;
        assert_eq!(
            app.list_host_dns_configuration(host, HostSessionId::new())
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert!(transport.calls.lock().unwrap().is_empty());
        let audit = std::fs::read(dir.path().join("audit.jsonl")).unwrap_or_default();
        let hosts = app.list_hosts().unwrap();
        let _services = app.service_limit.acquire_many(4).await.unwrap();
        let snapshot = app.list_host_dns_configuration(host, owner).await.unwrap();
        assert_eq!(snapshot.host_id, host);
        assert_eq!(snapshot.host_session_id, owner);
        assert_eq!(snapshot.nameservers[0].address, "127.0.0.53");
        assert_eq!(
            *transport.calls.lock().unwrap(),
            vec![ReadOnlyCommand::DnsConfiguration]
        );
        assert_eq!(app.list_hosts().unwrap(), hosts);
        assert_eq!(
            std::fs::read(dir.path().join("audit.jsonl")).unwrap_or_default(),
            audit
        );
        assert_eq!(app.network_limit.available_permits(), 4);
    }

    #[tokio::test]
    async fn all_four_network_operation_types_share_the_four_global_permits() {
        let (_dir, app, _) = setup(None);
        let mut requests = Vec::new();
        let mut transports = Vec::new();
        for command in [
            ReadOnlyCommand::NetworkAddresses,
            ReadOnlyCommand::NetworkRoutesV4,
            ReadOnlyCommand::ListeningPorts,
            ReadOnlyCommand::DnsConfiguration,
        ] {
            let transport = transport(Some(command));
            let (host, owner) = install(&app, transport.clone()).await;
            let request_app = app.clone();
            requests.push(tokio::spawn(async move {
                match command {
                    ReadOnlyCommand::NetworkAddresses => {
                        request_app.list_host_network(host, owner).await.map(|_| ())
                    }
                    ReadOnlyCommand::NetworkRoutesV4 => {
                        request_app.list_host_routes(host, owner).await.map(|_| ())
                    }
                    ReadOnlyCommand::ListeningPorts => request_app
                        .list_host_listening_ports(host, owner)
                        .await
                        .map(|_| ()),
                    ReadOnlyCommand::DnsConfiguration => request_app
                        .list_host_dns_configuration(host, owner)
                        .await
                        .map(|_| ()),
                    _ => unreachable!(),
                }
            }));
            entered(&transport).await;
            transports.push(transport);
        }
        assert_eq!(app.network_limit.available_permits(), 0);
        let fifth = transport(None);
        let (host, owner) = install(&app, fifth).await;
        assert_eq!(
            app.list_host_dns_configuration(host, owner)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        for transport in &transports {
            transport.release.add_permits(1);
        }
        for request in requests {
            assert!(request.await.unwrap().is_ok());
        }
        assert_eq!(app.network_limit.available_permits(), 4);
    }
    #[tokio::test]
    async fn listening_ports_share_the_four_global_network_slots() {
        let (_dir, app, transport) = setup(Some(ReadOnlyCommand::ListeningPorts));
        let mut requests = vec![];
        for _ in 0..4 {
            let (host, owner) = install(&app, transport.clone()).await;
            let request_app = app.clone();
            requests.push(tokio::spawn(async move {
                request_app.list_host_listening_ports(host, owner).await
            }));
            entered(&transport).await;
        }
        let (host, owner) = install(&app, transport.clone()).await;
        assert_eq!(
            app.list_host_listening_ports(host, owner)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            app.list_host_network(host, owner).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(
            app.list_host_routes(host, owner).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(
            app.list_host_dns_configuration(host, owner)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        transport.release.add_permits(4);
        for request in requests {
            assert!(request.await.unwrap().is_ok());
        }
        assert_eq!(app.network_limit.available_permits(), 4);
    }
    #[tokio::test]
    async fn listening_ports_reject_post_io_generation_transport_and_view_replacement() {
        for change in 0..4 {
            let (_dir, app, transport) = setup(Some(ReadOnlyCommand::ListeningPorts));
            let (host, owner) = install(&app, transport.clone()).await;
            let request_app = app.clone();
            let request =
                tokio::spawn(
                    async move { request_app.list_host_listening_ports(host, owner).await },
                );
            entered(&transport).await;
            let slot = app.slot(host).await;
            {
                let mut data = slot.data.lock().await;
                match change {
                    0 => data.generation += 1,
                    1 => data.connection_id = Some(HostSessionId::new()),
                    2 => data.view.host_session_id = Some(HostSessionId::new()),
                    _ => data.view.state = ConnectionState::Disconnected,
                }
            }
            transport.release.add_permits(1);
            assert_eq!(
                request.await.unwrap().unwrap_err().code,
                ErrorCode::Cancelled
            );
            assert_eq!(app.network_limit.available_permits(), 4);
        }
    }
    #[tokio::test]
    async fn listening_ports_cancellation_and_transport_failure_release_admission_without_retry() {
        let (_dir, app, transport) = setup(Some(ReadOnlyCommand::ListeningPorts));
        let (host, owner) = install(&app, transport.clone()).await;
        let request_app = app.clone();
        let request =
            tokio::spawn(async move { request_app.list_host_listening_ports(host, owner).await });
        entered(&transport).await;
        app.slot(host).await.data.lock().await.cancel.cancel();
        assert_eq!(
            request.await.unwrap().unwrap_err().code,
            ErrorCode::Cancelled
        );
        assert_eq!(app.network_limit.available_permits(), 4);
        let (_dir, app, transport) = setup(None);
        let (host, owner) = install(&app, transport.clone()).await;
        transport
            .fail
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            app.list_host_listening_ports(host, owner)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Connection
        );
        assert_eq!(transport.calls.lock().unwrap().len(), 1);
        assert_eq!(app.network_limit.available_permits(), 4);
        transport
            .fail
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(app.list_host_listening_ports(host, owner).await.is_ok());
    }

    #[tokio::test]
    async fn dns_rejects_stale_post_io_completion_and_releases_admission_without_retry() {
        let (_dir, app, transport) = setup(Some(ReadOnlyCommand::DnsConfiguration));
        let (host, owner) = install(&app, transport.clone()).await;
        let request_app = app.clone();
        let request =
            tokio::spawn(async move { request_app.list_host_dns_configuration(host, owner).await });
        entered(&transport).await;
        app.slot(host).await.data.lock().await.generation += 1;
        transport.release.add_permits(1);
        assert_eq!(
            request.await.unwrap().unwrap_err().code,
            ErrorCode::Cancelled
        );
        assert_eq!(transport.calls.lock().unwrap().len(), 1);
        assert_eq!(app.network_limit.available_permits(), 4);

        let (_dir, app, transport) = setup(None);
        let (host, owner) = install(&app, transport.clone()).await;
        transport
            .fail
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            app.list_host_dns_configuration(host, owner)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Connection
        );
        assert_eq!(transport.calls.lock().unwrap().len(), 1);
        assert_eq!(app.network_limit.available_permits(), 4);
    }
}
