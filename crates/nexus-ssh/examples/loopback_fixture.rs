//! TEST ONLY: a disposable, loopback-only SSH server for the real desktop flow.
//! All discovery responses are fixed text. No command is ever passed to a shell.

#[path = "../tests/support/probe_responses.rs"]
mod probe_responses;

use std::{
    io::{self, Write},
    sync::Arc,
    time::Duration,
};

use russh::{
    Channel, ChannelId,
    keys::{Algorithm, HashAlg, PrivateKey},
    server::{self, Auth, Server as _},
};
use tokio::net::TcpListener;

const USERNAME: &str = "nexusops";
const PASSWORD: &str = "fixture-only";

#[derive(Clone)]
struct FixtureServer;

impl server::Server for FixtureServer {
    type Handler = Self;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        Self
    }
}

impl server::Handler for FixtureServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == USERNAME && password == PASSWORD {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        command: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if let Some(response) = probe_responses::response(command) {
            session.channel_success(channel)?;
            session.data(channel, response.as_bytes())?;
            session.exit_status_request(channel, 0)?;
            session.eof(channel)?;
        } else {
            session.channel_failure(channel)?;
            session.exit_status_request(channel, 126)?;
        }
        session.close(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)?;
        session.close(channel)?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (port, lifetime_seconds) = arguments()?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    // A fresh key on every launch allows rotation testing with --port <previous port>.
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)?;
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::from_millis(20),
        auth_rejection_time_initial: Some(Duration::from_millis(20)),
        inactivity_timeout: Some(Duration::from_secs(120)),
        ..Default::default()
    });
    let mut server = FixtureServer;
    let running = server.run_on_socket(config, &listener);
    let shutdown = running.handle();
    // These are deliberately public fixture credentials, never application secrets.
    let announcement = serde_json::json!({
        "testOnly": true,
        "hostname": "127.0.0.1",
        "port": listener.local_addr()?.port(),
        "username": USERNAME,
        "password": PASSWORD,
        "algorithm": "ssh-ed25519",
        "fingerprint": fingerprint,
        "pid": std::process::id(),
        "lifetimeSeconds": lifetime_seconds,
    });
    {
        let mut output = io::stdout().lock();
        serde_json::to_writer(&mut output, &announcement)?;
        writeln!(output)?;
        output.flush()?;
    }
    tokio::select! {
        result = running => result?,
        _ = tokio::time::sleep(Duration::from_secs(lifetime_seconds)) => shutdown.shutdown("test fixture lifetime ended".into()),
    }
    Ok(())
}

fn arguments() -> Result<(u16, u64), io::Error> {
    let mut port = 0u16;
    let mut lifetime_seconds = 3600u64;
    let mut arguments = std::env::args().skip(1);
    while let Some(option) = arguments.next() {
        let value = arguments.next().ok_or_else(usage)?;
        match option.as_str() {
            "--port" => port = value.parse().map_err(|_| usage())?,
            "--lifetime-seconds" => lifetime_seconds = value.parse().map_err(|_| usage())?,
            _ => return Err(usage()),
        }
    }
    if !(1..=86_400).contains(&lifetime_seconds) {
        return Err(usage());
    }
    Ok((port, lifetime_seconds))
}

fn usage() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "TEST ONLY: loopback_fixture [--port 0..65535] [--lifetime-seconds 1..86400]; binds 127.0.0.1 only",
    )
}
