use crate::{KnownHosts, error::HandlerError};
use nexus_model::{AppError, ErrorCode, HostFingerprint};
use russh::{
    client,
    keys::{HashAlg, PublicKeyOrCertificate},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
pub(crate) struct Client {
    pub(crate) known_hosts: Arc<KnownHosts>,
    pub(crate) hostname: String,
    pub(crate) port: u16,
    pub(crate) closed: Arc<AtomicBool>,
}

impl client::Handler for Client {
    type Error = HandlerError;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // CA/certificate semantics are deliberately not inferred from a raw-key pin.
        if server_key.certificate().is_some() {
            return Err(HandlerError::Application(AppError::new(
                ErrorCode::Policy,
                "SSH host certificates are not supported. Configure a raw host key.",
            )));
        }
        let public_key = server_key.public_key();
        let fingerprint = HostFingerprint {
            algorithm: public_key.algorithm().to_string(),
            sha256: public_key.fingerprint(HashAlg::Sha256).to_string(),
        };
        self.known_hosts
            .verify(&self.hostname, self.port, &fingerprint)
            .map_err(HandlerError::Application)?;
        Ok(true)
    }

    async fn disconnected(
        &mut self,
        reason: client::DisconnectReason<Self::Error>,
    ) -> Result<(), Self::Error> {
        self.closed.store(true, Ordering::Release);
        match reason {
            client::DisconnectReason::Error(error) => Err(error),
            client::DisconnectReason::ReceivedDisconnect(_) => Ok(()),
        }
    }
}
