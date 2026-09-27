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

#[cfg(test)]
mod tests {
    use super::*;
    use russh::{
        client::Handler,
        keys::{Algorithm, PrivateKey, ssh_key::certificate},
    };

    #[tokio::test]
    async fn host_certificate_is_rejected_even_when_its_raw_subject_key_is_pinned() {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let ca = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let mut builder = certificate::Builder::new_with_random_nonce(
            &mut rand::rng(),
            key.public_key(),
            0,
            u64::MAX,
        )
        .unwrap();
        builder.cert_type(certificate::CertType::Host).unwrap();
        builder.valid_principal("host").unwrap();
        let certificate = builder.sign(&ca).unwrap();
        let known_hosts = Arc::new(KnownHosts::open(":memory:").unwrap());
        known_hosts
            .trust(&nexus_model::HostKeyChallenge {
                hostname: "host".into(),
                port: 22,
                algorithm: key.algorithm().to_string(),
                fingerprint: key.public_key().fingerprint(HashAlg::Sha256).to_string(),
                previous_fingerprint: None,
            })
            .unwrap();
        let mut client = Client {
            known_hosts,
            hostname: "host".into(),
            port: 22,
            closed: Arc::new(AtomicBool::new(false)),
        };
        let error = client
            .check_server_key(&certificate.into())
            .await
            .unwrap_err()
            .into_app();
        assert_eq!(error.code, ErrorCode::Policy);
        assert!(error.host_key.is_none());
        assert!(
            client
                .check_server_key(&key.public_key().clone().into())
                .await
                .unwrap()
        );
    }
}
