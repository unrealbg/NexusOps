use super::*;

#[tokio::test]
async fn logs_reads_do_not_append_audit_or_change_metadata_persistence() {
    let (dir, app, _) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let id = app
        .get_session(host.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    let audit_before = std::fs::read(dir.path().join("audit.jsonl")).unwrap();
    let metadata_before = std::fs::read(dir.path().join("hosts.db")).unwrap();
    let snapshot = app.list_host_logs(host.id, id).await.unwrap();
    assert_eq!(snapshot.entries.len(), 1);
    assert_eq!(
        std::fs::read(dir.path().join("audit.jsonl")).unwrap(),
        audit_before
    );
    assert_eq!(
        std::fs::read(dir.path().join("hosts.db")).unwrap(),
        metadata_before
    );
}

#[tokio::test]
async fn logs_disconnected_and_stale_generation_fail_closed() {
    let (_dir, app, provider) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    assert_eq!(
        app.list_host_logs(host.id, HostSessionId::new())
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    app.connect_host(host.id).await.unwrap();
    let id = app
        .get_session(host.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    provider.logs.stall.store(true, Ordering::SeqCst);
    let other = app.clone();
    let request = tokio::spawn(async move { other.list_host_logs(host.id, id).await });
    provider.logs.entered.acquire().await.unwrap().forget();
    // Invalidate only the generation: the completed transport response itself is valid.
    app.slot(host.id).await.data.lock().await.generation += 1;
    provider.logs.release.add_permits(1);
    assert_eq!(
        request.await.unwrap().unwrap_err().code,
        ErrorCode::Cancelled
    );
}

#[tokio::test]
async fn logs_admission_is_independent_from_services_and_network_both_directions() {
    let (_dir, app, provider) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let id = app
        .get_session(host.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    let services = app.service_limit.acquire_many(4).await.unwrap();
    let network = app.network_limit.acquire_many(4).await.unwrap();
    assert!(app.list_host_logs(host.id, id).await.is_ok());
    drop((services, network));
    provider.logs.stall.store(true, Ordering::SeqCst);
    let other = app.clone();
    let request = tokio::spawn(async move { other.list_host_logs(host.id, id).await });
    provider.logs.entered.acquire().await.unwrap().forget();
    let remaining = app.log_limit.acquire_many(3).await.unwrap();
    assert!(app.list_host_services(host.id, id).await.is_ok());
    assert!(app.list_host_network(host.id, id).await.is_ok());
    drop(remaining);
    provider.logs.release.add_permits(1);
    assert!(request.await.unwrap().is_ok());
}

#[tokio::test]
async fn logs_timeout_releases_admission_for_later_refresh() {
    let (_dir, app, provider) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let id = app
        .get_session(host.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    provider.logs.stall.store(true, Ordering::SeqCst);
    assert_eq!(
        app.list_host_logs(host.id, id).await.unwrap_err().code,
        ErrorCode::Timeout
    );
    provider.logs.stall.store(false, Ordering::SeqCst);
    assert!(app.list_host_logs(host.id, id).await.is_ok());
    assert_eq!(app.log_limit.available_permits(), 4);
}

#[tokio::test]
async fn logs_requires_exact_session_keeps_hosts_independent_and_cleans_deleted_gate() {
    let (_dir, app, _) = setup(false);
    let first = app.save_host(input(), Some(credential())).await.unwrap();
    let second = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(first.id).await.unwrap();
    app.connect_host(second.id).await.unwrap();
    let first_session = app
        .get_session(first.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    let second_session = app
        .get_session(second.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    assert_eq!(
        app.list_host_logs(first.id, second_session)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let first_snapshot = app.list_host_logs(first.id, first_session).await.unwrap();
    let second_snapshot = app.list_host_logs(second.id, second_session).await.unwrap();
    assert_eq!(first_snapshot.host_session_id, first_session);
    assert_eq!(
        first_snapshot.entries[0].message.as_deref(),
        Some("synthetic journal message")
    );
    assert_eq!(second_snapshot.host_id, second.id);
    assert_eq!(app.log_gates.lock().await.len(), 2);
    app.delete_host(first.id).await.unwrap();
    assert!(!app.log_gates.lock().await.contains_key(&first.id));
    assert!(app.list_host_logs(second.id, second_session).await.is_ok());
}

#[tokio::test]
async fn logs_busy_request_disconnect_and_reconnect_cannot_publish_old_session() {
    let (_dir, app, provider) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let old_session = app
        .get_session(host.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    provider.logs.stall.store(true, Ordering::SeqCst);
    let request_app = app.clone();
    let request =
        tokio::spawn(async move { request_app.list_host_logs(host.id, old_session).await });
    provider.logs.entered.acquire().await.unwrap().forget();
    assert_eq!(
        app.list_host_logs(host.id, old_session)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    app.disconnect_host(host.id).await.unwrap();
    provider.logs.release.add_permits(1);
    assert_eq!(
        request.await.unwrap().unwrap_err().code,
        ErrorCode::Cancelled
    );
    provider.logs.stall.store(false, Ordering::SeqCst);
    app.connect_host(host.id).await.unwrap();
    let new_session = app
        .get_session(host.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    assert_ne!(old_session, new_session);
    assert_eq!(
        app.list_host_logs(host.id, old_session)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert!(app.list_host_logs(host.id, new_session).await.is_ok());
}

#[tokio::test]
async fn logs_global_limit_is_non_queuing_and_shutdown_clears_gates() {
    let (_dir, app, provider) = setup(false);
    let mut hosts = Vec::new();
    for _ in 0..5 {
        let host = app.save_host(input(), Some(credential())).await.unwrap();
        app.connect_host(host.id).await.unwrap();
        let session = app
            .get_session(host.id)
            .await
            .unwrap()
            .host_session_id
            .unwrap();
        hosts.push((host.id, session));
    }
    provider.logs.stall.store(true, Ordering::SeqCst);
    let requests = hosts[..4]
        .iter()
        .map(|(host, session)| {
            let app = app.clone();
            let (host, session) = (*host, *session);
            tokio::spawn(async move { app.list_host_logs(host, session).await })
        })
        .collect::<Vec<_>>();
    for _ in 0..4 {
        provider.logs.entered.acquire().await.unwrap().forget();
    }
    assert_eq!(
        app.list_host_logs(hosts[4].0, hosts[4].1)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    provider.logs.release.add_permits(4);
    for request in requests {
        assert!(request.await.unwrap().is_ok());
    }
    app.shutdown().await.unwrap();
    assert!(app.log_gates.lock().await.is_empty());
    assert_eq!(
        app.list_host_logs(hosts[0].0, hosts[0].1)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}

#[tokio::test]
async fn logs_remote_closure_edit_and_mutation_admission_reject_old_authority() {
    let (_dir, app, provider) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let old_session = app
        .get_session(host.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    let mutation = app.mutation.lock().await;
    assert_eq!(
        app.list_host_logs(host.id, old_session)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    drop(mutation);
    provider.logs.stall.store(true, Ordering::SeqCst);
    let request_app = app.clone();
    let request =
        tokio::spawn(async move { request_app.list_host_logs(host.id, old_session).await });
    provider.logs.entered.acquire().await.unwrap().forget();
    app.slot(host.id).await.data.lock().await.cancel.cancel();
    assert_eq!(
        app.get_session(host.id).await.unwrap().state,
        ConnectionState::Failed
    );
    provider.logs.release.add_permits(1);
    assert_eq!(
        request.await.unwrap().unwrap_err().code,
        ErrorCode::Cancelled
    );
    provider.logs.stall.store(false, Ordering::SeqCst);
    app.disconnect_host(host.id).await.unwrap();
    let mut edited = input();
    edited.id = Some(host.id);
    edited.display_name = "Renamed host".into();
    app.save_host(edited, None).await.unwrap();
    assert_eq!(
        app.list_host_logs(host.id, old_session)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    app.connect_host(host.id).await.unwrap();
    let next_session = app
        .get_session(host.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    assert_ne!(old_session, next_session);
    assert!(app.list_host_logs(host.id, next_session).await.is_ok());
}

#[tokio::test]
async fn deleting_host_during_logs_read_revokes_result_and_removes_gate() {
    let (_dir, app, provider) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let session = app
        .get_session(host.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    provider.logs.stall.store(true, Ordering::SeqCst);
    let request_app = app.clone();
    let request = tokio::spawn(async move { request_app.list_host_logs(host.id, session).await });
    provider.logs.entered.acquire().await.unwrap().forget();
    app.delete_host(host.id).await.unwrap();
    assert!(!app.log_gates.lock().await.contains_key(&host.id));
    provider.logs.release.add_permits(1);
    assert_eq!(
        request.await.unwrap().unwrap_err().code,
        ErrorCode::Cancelled
    );
    assert_eq!(
        app.list_host_logs(host.id, session).await.unwrap_err().code,
        ErrorCode::NotFound
    );
}
