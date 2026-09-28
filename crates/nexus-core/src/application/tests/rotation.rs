use super::*;

fn fingerprint(ch: char) -> HostFingerprint {
    HostFingerprint {
        algorithm: "ssh-ed25519".into(),
        sha256: format!("SHA256:{}", ch.to_string().repeat(43)),
    }
}

async fn fixture(forbid_io: bool) -> (tempfile::TempDir, Arc<Application>, Host) {
    let (dir, mut app, _) = setup(false);
    if forbid_io {
        let writable = Arc::get_mut(&mut app).expect("unique application");
        writable.secrets = Arc::new(security::Forbidden);
        writable.provider = Arc::new(security::Forbidden);
    }
    let host = input().into_host().expect("host");
    app.repository
        .save(&StoredHost {
            host: host.clone(),
            credential_id: HostId::new(),
        })
        .expect("save host metadata");
    let old = fingerprint('A');
    let unknown = app
        .known_hosts
        .verify("localhost", 22, &old)
        .expect_err("first contact")
        .host_key
        .expect("challenge");
    app.known_hosts.trust(&unknown).expect("initial pin");
    set_changed(&app, host.id, fingerprint('B')).await;
    (dir, app, host)
}

async fn set_changed(app: &Application, id: HostId, presented: HostFingerprint) {
    let error = app
        .known_hosts
        .verify("localhost", 22, &presented)
        .expect_err("changed key");
    assert_eq!(error.code, ErrorCode::ChangedHostKey);
    let slot = app.slot(id).await;
    let mut data = slot.data.lock().await;
    data.view = HostSession::disconnected(id);
    data.view.state = ConnectionState::Failed;
    data.view.error = Some(error);
}

#[tokio::test]
async fn plan_and_execute_are_offline_one_shot_and_audit_only_metadata() {
    let (dir, app, host) = fixture(true).await;
    let before = std::fs::read(dir.path().join("audit.jsonl")).unwrap();
    let plan = app.plan_host_key_rotation(host.id).await.expect("plan");
    assert_eq!(plan.host_id, host.id);
    assert_eq!(plan.hostname, "localhost");
    assert_eq!(plan.port, 22);
    assert_eq!(plan.current_fingerprint, fingerprint('A'));
    assert_eq!(plan.presented_fingerprint, fingerprint('B'));
    assert_eq!(
        std::fs::read(dir.path().join("audit.jsonl")).unwrap(),
        before
    );
    assert_eq!(
        app.known_hosts.fingerprint("localhost", 22).unwrap(),
        Some(fingerprint('A'))
    );
    let superseded = app
        .plan_host_key_rotation(host.id)
        .await
        .expect("replacement plan");
    assert_ne!(plan.id, superseded.id);
    assert_eq!(
        app.execute_host_key_rotation(host.id, plan.id)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    app.execute_host_key_rotation(host.id, superseded.id)
        .await
        .expect("rotate");
    assert_eq!(
        app.known_hosts.fingerprint("localhost", 22).unwrap(),
        Some(fingerprint('B'))
    );
    let session = app.get_session(host.id).await.expect("session");
    assert_eq!(session.state, ConnectionState::Disconnected);
    assert!(session.host_session_id.is_none());
    assert!(session.identity.is_none());
    assert!(session.error.is_none());
    assert_eq!(
        app.execute_host_key_rotation(host.id, superseded.id)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let events = audit_events(dir.path());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].operation.kind, "identity.rotate");
    assert_eq!(events[0].operation.risk, OperationRisk::High);
    assert_eq!(events[0].outcome, AuditOutcome::Success);
    let audit_text = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
    for forbidden in [
        "localhost",
        "ssh-ed25519",
        &fingerprint('A').sha256,
        &fingerprint('B').sha256,
    ] {
        assert!(!audit_text.contains(forbidden));
    }
    // Panic-on-use SecretStore and ConnectionProvider prove the entire plan/execute path
    // neither accesses credentials nor establishes a transport.
}

#[tokio::test]
async fn planning_rejects_every_non_changed_session_and_missing_or_mismatched_challenge() {
    let (_dir, app, host) = fixture(true).await;
    let slot = app.slot(host.id).await;
    let original = slot.data.lock().await.view.error.clone().unwrap();
    for state in [
        ConnectionState::Disconnected,
        ConnectionState::Connecting,
        ConnectionState::AwaitingTrust,
        ConnectionState::Connected,
    ] {
        slot.data.lock().await.view.state = state;
        assert_eq!(
            app.plan_host_key_rotation(host.id).await.unwrap_err().code,
            ErrorCode::Conflict
        );
    }
    slot.data.lock().await.view.state = ConnectionState::Failed;
    for error in [
        None,
        Some(AppError::new(ErrorCode::Connection, "failure")),
        Some(AppError {
            host_key: None,
            ..original.clone()
        }),
        Some(AppError {
            host_key: Some(Box::new(HostKeyChallenge {
                previous_fingerprint: None,
                ..*original.host_key.clone().unwrap()
            })),
            ..original.clone()
        }),
        Some(AppError {
            host_key: Some(Box::new(HostKeyChallenge {
                hostname: "other".into(),
                ..*original.host_key.clone().unwrap()
            })),
            ..original.clone()
        }),
        Some(AppError {
            host_key: Some(Box::new(HostKeyChallenge {
                port: 2222,
                ..*original.host_key.clone().unwrap()
            })),
            ..original.clone()
        }),
        Some(AppError {
            host_key: Some(Box::new(HostKeyChallenge {
                previous_fingerprint: Some(fingerprint('C').sha256),
                ..*original.host_key.clone().unwrap()
            })),
            ..original.clone()
        }),
    ] {
        slot.data.lock().await.view.error = error;
        assert_eq!(
            app.plan_host_key_rotation(host.id).await.unwrap_err().code,
            ErrorCode::Conflict
        );
    }
    slot.data.lock().await.view.error = Some(original);
    app.known_hosts
        .rotate("localhost", 22, &fingerprint('A'), &fingerprint('C'))
        .unwrap();
    assert_eq!(
        app.plan_host_key_rotation(host.id).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        app.plan_host_key_rotation(HostId::new())
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}

#[tokio::test]
async fn execution_consumes_expired_changed_or_raced_authority() {
    for scenario in ["expired", "generation", "challenge", "pin", "host"] {
        let (dir, app, host) = fixture(true).await;
        let plan = app.plan_host_key_rotation(host.id).await.unwrap();
        match scenario {
            "expired" => {
                app.rotation_plans
                    .lock()
                    .await
                    .get_mut(&host.id)
                    .unwrap()
                    .expires_at = Instant::now()
            }
            "generation" => app.slot(host.id).await.data.lock().await.generation += 1,
            "challenge" => set_changed(&app, host.id, fingerprint('C')).await,
            "pin" => app
                .known_hosts
                .rotate("localhost", 22, &fingerprint('A'), &fingerprint('C'))
                .unwrap(),
            "host" => {
                let mut edited = host.clone();
                edited.connection.hostname = "other.example".into();
                app.repository
                    .save(&StoredHost {
                        host: edited,
                        credential_id: HostId::new(),
                    })
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(
            app.execute_host_key_rotation(host.id, plan.id)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict,
            "{scenario}"
        );
        assert_eq!(
            app.execute_host_key_rotation(host.id, plan.id)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        let expected = if scenario == "pin" {
            fingerprint('C')
        } else {
            fingerprint('A')
        };
        assert_eq!(
            app.known_hosts.fingerprint("localhost", 22).unwrap(),
            Some(expected)
        );
        let events = audit_events(dir.path());
        assert_eq!(events.len(), 1, "{scenario}");
        assert_eq!(events[0].outcome, AuditOutcome::Failed);
        assert_eq!(events[0].operation.kind, "identity.rotate");
    }
}

#[tokio::test]
async fn wrong_host_id_cannot_use_another_hosts_plan_and_delete_invalidates_it() {
    let (_dir, app, host) = fixture(false).await;
    let plan = app.plan_host_key_rotation(host.id).await.unwrap();
    assert_eq!(
        app.execute_host_key_rotation(HostId::new(), plan.id)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    app.delete_host(host.id).await.expect("delete");
    assert_eq!(
        app.execute_host_key_rotation(host.id, plan.id)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        app.known_hosts.fingerprint("localhost", 22).unwrap(),
        Some(fingerprint('A'))
    );
}

#[tokio::test]
async fn ordinary_host_edit_and_new_connect_attempt_invalidate_old_authority() {
    let (_dir, app, host) = fixture(false).await;
    let before_edit = app.plan_host_key_rotation(host.id).await.unwrap();
    let mut edit = input();
    edit.id = Some(host.id);
    edit.display_name = "Renamed host".into();
    app.save_host(edit, None).await.expect("metadata edit");
    assert_eq!(
        app.execute_host_key_rotation(host.id, before_edit.id)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    set_changed(&app, host.id, fingerprint('B')).await;
    let before_connect = app.plan_host_key_rotation(host.id).await.unwrap();
    // The fixture has no actual credential row; the normal connect attempt fails after
    // invalidating rotation authority, without making a network connection.
    assert!(app.connect_host(host.id).await.is_err());
    assert_eq!(
        app.execute_host_key_rotation(host.id, before_connect.id)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        app.known_hosts.fingerprint("localhost", 22).unwrap(),
        Some(fingerprint('A'))
    );
}

#[tokio::test]
async fn missing_pin_and_invalid_old_storage_never_issue_a_rotation_plan() {
    let (dir, app, host) = fixture(true).await;
    let db = rusqlite::Connection::open(dir.path().join("pins.db")).unwrap();
    db.execute("DELETE FROM ssh_host_keys", []).unwrap();
    assert_eq!(
        app.plan_host_key_rotation(host.id).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    db.execute("INSERT INTO ssh_host_keys (hostname, port, algorithm, fingerprint) VALUES ('localhost',22,'ssh-ed25519','invalid')", []).unwrap();
    assert_eq!(
        app.plan_host_key_rotation(host.id).await.unwrap_err().code,
        ErrorCode::Persistence
    );
}

#[tokio::test]
async fn audit_failure_after_local_replacement_is_reported_without_pin_rollback() {
    let (dir, app, host) = fixture(true).await;
    let plan = app.plan_host_key_rotation(host.id).await.unwrap();
    let audit_path = dir.path().join("audit.jsonl");
    std::fs::remove_file(&audit_path).unwrap();
    std::fs::create_dir(&audit_path).unwrap();
    let error = app
        .execute_host_key_rotation(host.id, plan.id)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Persistence);
    assert_eq!(error.message, "The local audit record could not be saved.");
    assert_eq!(
        app.known_hosts.fingerprint("localhost", 22).unwrap(),
        Some(fingerprint('B'))
    );
    let session = app.get_session(host.id).await.unwrap();
    assert_eq!(session.state, ConnectionState::Disconnected);
    assert!(session.host_session_id.is_none());
    assert_eq!(
        app.execute_host_key_rotation(host.id, plan.id)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}
