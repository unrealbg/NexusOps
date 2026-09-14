use crate::{
    KnownHosts,
    bounded_io::{CancellableStream, bounded},
    error::{HandlerError, io_error, transport_error},
    handler::Client,
    known_hosts::canonical_endpoint,
    session::{OUTPUT_LIMIT, SshSession},
};
use nexus_model::{AppError, AuthenticationMethod, ErrorCode, Host};
use nexus_secrets::Credential;
use russh::{
    client,
    keys::{PrivateKeyWithHashAlg, decode_secret_key},
};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use tokio::{
    net::{TcpStream, lookup_host},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const AUTH_TIMEOUT: Duration = Duration::from_secs(30);
/// Creates independent SSH sessions; only immutable configuration and host pins are shared.
pub struct SshProvider {
    known_hosts: Arc<KnownHosts>,
}

impl SshProvider {
    pub fn new(known_hosts: Arc<KnownHosts>) -> Self {
        Self { known_hosts }
    }

    /// Complete host verification before transmitting authentication material.
    /// Cancel the supplied token to close this session, including after connect returns.
    pub async fn connect(
        &self,
        host: &Host,
        credential: Credential,
        cancellation: CancellationToken,
    ) -> Result<Arc<SshSession>, AppError> {
        credential.validate(host.connection.authentication)?;
        let endpoint = canonical_endpoint(&host.connection.hostname, host.connection.port)?;
        let lifetime = cancellation.child_token();
        let guard = lifetime.clone().drop_guard();
        let closed = Arc::new(AtomicBool::new(false));
        let handler = Client {
            known_hosts: self.known_hosts.clone(),
            hostname: endpoint.clone(),
            port: host.connection.port,
            closed: closed.clone(),
        };
        let config = Arc::new(client::Config {
            // Bound unconsumed remote packets as well as the collected output.
            window_size: OUTPUT_LIMIT as u32,
            maximum_packet_size: 16 * 1024,
            channel_buffer_size: 4,
            keepalive_interval: Some(Duration::from_secs(20)),
            keepalive_max: 3,
            ..Default::default()
        });
        let mut handle = bounded(&lifetime, CONNECT_TIMEOUT, "connect", async {
            let addresses: Vec<_> = lookup_host((endpoint.as_str(), host.connection.port)).await
                .map_err(|error| {
                    tracing::warn!(stage = "dns", io_kind = ?error.kind(), "SSH address resolution failed");
                    AppError::new(ErrorCode::Dns, "The server hostname could not be resolved.")
                })?.collect();
            if addresses.is_empty() {
                return Err(AppError::new(ErrorCode::Dns, "The server hostname did not resolve to an address."));
            }
            let stream = TcpStream::connect(addresses.as_slice()).await.map_err(|error| io_error("connect", &error))?;
            stream.set_nodelay(true).map_err(|error| io_error("socket", &error))?;
            client::connect_stream(config, CancellableStream::new(stream, lifetime.clone()), handler).await.map_err(HandlerError::into_app)
        }).await?;

        bounded(&lifetime, AUTH_TIMEOUT, "authentication", async {
            let authenticated = match host.connection.authentication {
                AuthenticationMethod::Password => {
                    let password = credential.password.as_ref().ok_or_else(|| AppError::new(ErrorCode::Authentication, "No password is available for this host."))?;
                    handle.authenticate_password(host.connection.username.as_str(), password.as_str()).await
                }
                AuthenticationMethod::PrivateKey => {
                    let key = tokio::task::spawn_blocking(move || {
                        let private_key = credential.private_key.as_ref().ok_or_else(|| AppError::new(ErrorCode::Authentication, "No private key is available for this host."))?;
                        decode_secret_key(private_key.as_str(), credential.passphrase.as_ref().map(|value| value.as_str()))
                            .map_err(|_| AppError::new(ErrorCode::Authentication, "The private key could not be unlocked. Check its format and passphrase."))
                    }).await.map_err(|_| AppError::new(ErrorCode::Authentication, "The private key could not be loaded."))??;
                    let hash_algorithm = handle.best_supported_rsa_hash().await.map_err(|error| transport_error("authentication", &error))?.flatten();
                    handle.authenticate_publickey(host.connection.username.as_str(), PrivateKeyWithHashAlg::new(Arc::new(key), hash_algorithm)).await
                }
            }.map_err(|error| transport_error("authentication", &error))?;
            if !authenticated.success() {
                return Err(AppError::new(ErrorCode::Authentication, "SSH authentication was rejected. Check the username and credentials."));
            }
            Ok(())
        }).await?;
        tracing::info!(host_id = %host.id, stage = "authenticated", "SSH session established");
        Ok(Arc::new(SshSession {
            handle: Mutex::new(handle),
            lifetime,
            _guard: guard,
            closed,
        }))
    }
}
