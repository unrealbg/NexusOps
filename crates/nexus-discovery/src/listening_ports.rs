use nexus_model::{
    AppError, ErrorCode, HostId, HostSessionId, ListeningPortSnapshot, ListeningSocketEntry,
    ListeningSocketFamily as Family, ListeningSocketProtocol as Protocol,
    ListeningSocketState as State,
};
use nexus_operations::{OperationEngine, ReadOnlyCommand, RemoteSession};
use std::{collections::HashSet, net::IpAddr};
use tokio_util::sync::CancellationToken;

const MAX_OUTPUT: usize = 64 * 1024;
const MAX_LINE: usize = 512;
const MAX_ENDPOINT: usize = 128;
const MAX_ROWS: usize = 256;

fn invalid() -> AppError {
    AppError::new(
        ErrorCode::Discovery,
        "The host returned an invalid or unsupported listening-port inventory.",
    )
}

fn decimal(value: &str) -> Result<u64, AppError> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    value.parse().map_err(|_| invalid())
}

// A rightmost colon is the endpoint separator even for unbracketed IPv6.
// Brackets, when present, must enclose only an IPv6 address and optional zone.
fn endpoint(value: &str) -> Result<(&str, &str, bool), AppError> {
    if value.len() > MAX_ENDPOINT {
        return Err(invalid());
    }
    if let Some(rest) = value.strip_prefix('[') {
        let (address, port) = rest.split_once("]:").ok_or_else(invalid)?;
        Ok((address, port, true))
    } else {
        let (address, port) = value.rsplit_once(':').ok_or_else(invalid)?;
        Ok((address, port, false))
    }
}

fn address(value: &str, bracketed: bool) -> Result<(String, Family), AppError> {
    if value == "*" && !bracketed {
        return Ok((value.into(), Family::Unknown));
    }
    let (raw, scope) = match value.split_once('%') {
        Some((raw, scope)) => {
            if scope.is_empty()
                || scope.len() > 32
                || !scope
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
            {
                return Err(invalid());
            }
            if scope.bytes().all(|byte| byte.is_ascii_digit())
                && scope.parse::<u32>().ok().is_none_or(|index| index == 0)
            {
                return Err(invalid());
            }
            (raw, Some(scope))
        }
        None => (value, None),
    };
    if raw.len() > 64 {
        return Err(invalid());
    }
    let ip: IpAddr = raw.parse().map_err(|_| invalid())?;
    let family = match ip {
        IpAddr::V4(_) if !bracketed && scope.is_none() => Family::Ipv4,
        IpAddr::V6(_) => Family::Ipv6,
        _ => return Err(invalid()),
    };
    let scope = match scope {
        Some(value) if value.bytes().all(|byte| byte.is_ascii_digit()) => {
            Some(value.parse::<u32>().map_err(|_| invalid())?.to_string())
        }
        Some(value) => Some(value.to_owned()),
        None => None,
    };
    let canonical = match scope {
        Some(scope) => format!("{ip}%{scope}"),
        None => ip.to_string(),
    };
    Ok((canonical, family))
}

fn local_and_peer(local: &str, peer: &str) -> Result<(String, Family, u16), AppError> {
    let (raw, port, bracketed) = endpoint(local)?;
    let (mut local_address, mut family) = address(raw, bracketed)?;
    let port = u16::try_from(decimal(port)?).map_err(|_| invalid())?;
    if port == 0 {
        return Err(invalid());
    }
    let (raw_peer, peer_port, bracketed_peer) = endpoint(peer)?;
    if !matches!(peer_port, "*" | "0") || raw_peer.contains('%') {
        return Err(invalid());
    }
    let (peer_address, peer_family) = address(raw_peer, bracketed_peer)?;
    if !matches!(peer_address.as_str(), "*" | "0.0.0.0" | "::") {
        return Err(invalid());
    }
    if family != Family::Unknown && peer_family != Family::Unknown && family != peer_family {
        return Err(invalid());
    }
    // Explicit peer address syntax can establish family. '*' alone never does.
    if family == Family::Unknown && peer_family != Family::Unknown {
        family = peer_family;
        local_address = peer_address;
    }
    Ok((local_address, family, port))
}

/// Parse exactly six headerless `ss -H -lntu` columns. Queue/peer fields are
/// validated then discarded; process/extended output rejects the whole result.
pub fn parse_listening_ports(output: &str) -> Result<Vec<ListeningSocketEntry>, AppError> {
    if output.len() > MAX_OUTPUT
        || output
            .as_bytes()
            .iter()
            .enumerate()
            .any(|(index, byte)| *byte == b'\r' && output.as_bytes().get(index + 1) != Some(&b'\n'))
    {
        return Err(invalid());
    }
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for line in output.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.len() > MAX_LINE
            || line
                .bytes()
                .any(|byte| !(byte == b'\t' || (b' '..=b'~').contains(&byte)))
        {
            return Err(invalid());
        }
        let mut fields = line.split_ascii_whitespace();
        let Some(protocol) = fields.next() else {
            continue;
        };
        let state = fields.next().ok_or_else(invalid)?;
        let (protocol, state) = match (protocol, state) {
            ("tcp", "LISTEN") => (Protocol::Tcp, State::Listen),
            ("udp", "UNCONN") => (Protocol::Udp, State::Unconn),
            _ => return Err(invalid()),
        };
        decimal(fields.next().ok_or_else(invalid)?)?;
        decimal(fields.next().ok_or_else(invalid)?)?;
        let local = fields.next().ok_or_else(invalid)?;
        let peer = fields.next().ok_or_else(invalid)?;
        if fields.next().is_some() {
            return Err(invalid());
        }
        let (local_address, family, port) = local_and_peer(local, peer)?;
        let entry = ListeningSocketEntry {
            protocol,
            family,
            local_address,
            port,
            state,
        };
        if entries.len() == MAX_ROWS || !seen.insert(entry.clone()) {
            return Err(invalid());
        }
        entries.push(entry);
    }
    Ok(entries)
}

pub async fn observe_listening_ports(
    session: &dyn RemoteSession,
    cancellation: CancellationToken,
    host_id: HostId,
    host_session_id: HostSessionId,
) -> Result<ListeningPortSnapshot, AppError> {
    let engine = OperationEngine::default();
    let output = engine
        .execute(
            session,
            &engine.plan(ReadOnlyCommand::ListeningPorts),
            cancellation.clone(),
        )
        .await?;
    let entries = parse_listening_ports(&output)?;
    if cancellation.is_cancelled() {
        return Err(AppError::new(
            ErrorCode::Cancelled,
            "The listening-port observation was cancelled.",
        ));
    }
    Ok(ListeningPortSnapshot {
        host_id,
        host_session_id,
        observed_at: chrono::Utc::now().to_rfc3339(),
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn accepts_tcp_udp_ip_wildcards_scopes_and_unknown_family_without_inference() {
        let rows = parse_listening_ports("tcp LISTEN 0 128 127.0.0.1:22 0.0.0.0:*\nudp UNCONN 1 0 0.0.0.0:53 0.0.0.0:*\ntcp LISTEN 0 512 [::1]:443 [::]:*\nudp UNCONN 0 0 2001:0db8::1%eth0:5353 :::*\ntcp LISTEN 0 1 *:8080 *:*\ntcp LISTEN 0 1 [::]:8443 *:*\n").unwrap();
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0].protocol, Protocol::Tcp);
        assert_eq!(rows[0].state, State::Listen);
        assert_eq!(rows[1].protocol, Protocol::Udp);
        assert_eq!(rows[1].state, State::Unconn);
        assert_eq!(rows[2].local_address, "::1");
        assert_eq!(rows[3].local_address, "2001:db8::1%eth0");
        assert_eq!(rows[3].family, Family::Ipv6);
        assert_eq!(rows[4].family, Family::Unknown);
        assert_eq!(rows[4].local_address, "*");
        assert_eq!(rows[5].family, Family::Ipv6);
        assert!(parse_listening_ports("").unwrap().is_empty());
        assert!(parse_listening_ports(" \t\r\n").unwrap().is_empty());
    }

    #[test]
    fn explicit_peer_family_can_establish_a_wildcard_and_canonicalize_numeric_zones() {
        let rows = parse_listening_ports("tcp LISTEN 0 1 *:80 0.0.0.0:*\nudp UNCONN 0 0 *:53 [::]:0\nudp UNCONN 0 0 [2001:db8::1%0002]:65535 [::]:*\n").unwrap();
        assert_eq!(rows[0].family, Family::Ipv4);
        assert_eq!(rows[0].local_address, "0.0.0.0");
        assert_eq!(rows[1].family, Family::Ipv6);
        assert_eq!(rows[1].local_address, "::");
        assert_eq!(rows[2].local_address, "2001:db8::1%2");
        assert_eq!(rows[2].port, 65535);
    }

    fn bad(input: &str) {
        let error = parse_listening_ports(input).unwrap_err();
        assert_eq!(error.code, ErrorCode::Discovery);
        assert_eq!(
            error.message,
            "The host returned an invalid or unsupported listening-port inventory."
        );
    }

    #[test]
    fn rejects_headers_columns_states_protocols_queues_and_process_metadata() {
        for input in [
            "Netid State Recv-Q Send-Q Local Peer",
            "tcp LISTEN 0 1 192.0.2.1:22",
            "tcp LISTEN 0 1 192.0.2.1:22 *:* extra",
            "udp LISTEN 0 1 *:22 *:*",
            "tcp UNCONN 0 1 *:22 *:*",
            "tcp ESTAB 0 1 *:22 *:*",
            "tcp6 LISTEN 0 1 [::]:22 [::]:*",
            "raw UNCONN 0 1 *:22 *:*",
            "tcp LISTEN -1 1 *:22 *:*",
            "tcp LISTEN 0 1.5 *:22 *:*",
            "tcp LISTEN 18446744073709551616 1 *:22 *:*",
            "tcp LISTEN x 1 *:22 *:*",
            "tcp LISTEN 0 1 *:22 *:* users:((pid=1))",
        ] {
            bad(input);
        }
        assert!(
            parse_listening_ports("tcp LISTEN 18446744073709551615 18446744073709551615 *:1 *:*")
                .is_ok()
        );
    }

    #[test]
    fn rejects_malformed_ambiguous_endpoints_ports_and_scopes() {
        for local in [
            "*:0",
            "*:65536",
            "*:http",
            "*:-1",
            "*:+22",
            "*:22:33",
            "192.0.2.999:22",
            "host.invalid:22",
            "[192.0.2.1]:22",
            "[*]:22",
            "[::1]22",
            "[::1]:22junk",
            "[::1]:22:33",
            "::22",
            "192.0.2.1%eth0:22",
            "[::1%]:22",
            "[::1%eth/0]:22",
            "[::1%a%b]:22",
            "[::1%0]:22",
            "[::1%4294967296]:22",
        ] {
            bad(&format!("tcp LISTEN 0 1 {local} *:*"));
        }
        for peer in [
            "192.0.2.1:*",
            "*:1",
            "*:http",
            "[::%eth0]:*",
            "[::]:*junk",
            "[]:*",
        ] {
            bad(&format!("tcp LISTEN 0 1 *:22 {peer}"));
        }
        bad("tcp LISTEN 0 1 192.0.2.1:22 [::]:*");
        bad("tcp LISTEN 0 1 [::]:22 0.0.0.0:*");
    }

    #[test]
    fn rejects_controls_ansi_unicode_and_every_size_boundary() {
        for hostile in [
            "\0", "\x1b[31m", "\u{202e}", "\u{200b}", "\x0b", "\x0c", "\r",
        ] {
            bad(&format!("tcp LISTEN 0 1 192.0.2.1{hostile}:22 *:*"));
        }
        bad("tcp LISTEN 0 1 *:22 *:*\r");
        bad(&" ".repeat(MAX_LINE + 1));
        bad(&format!(
            "{}tcp LISTEN 0 1 *:22 *:*",
            "\n".repeat(MAX_OUTPUT)
        ));
        bad(&format!("tcp LISTEN 0 1 [::1%{}]:22 *:*", "a".repeat(33)));
        bad(&format!(
            "tcp LISTEN 0 1 {}:22 *:*",
            "a".repeat(MAX_ENDPOINT)
        ));
        assert!(
            parse_listening_ports(&"\n".repeat(MAX_OUTPUT))
                .unwrap()
                .is_empty()
        );
    }

    fn inventory(count: usize) -> String {
        (0..count)
            .map(|index| format!("tcp LISTEN 0 128 192.0.2.1:{} *:*\n", index + 1))
            .collect()
    }
    #[test]
    fn bounds_rows_and_rejects_canonical_duplicates_but_preserves_distinct_observations() {
        assert_eq!(parse_listening_ports(&inventory(256)).unwrap().len(), 256);
        bad(&inventory(257));
        for input in [
            "tcp LISTEN 0 1 192.0.2.1:80 *:*\ntcp LISTEN 1 5 192.0.2.1:080 *:*",
            "tcp LISTEN 0 1 [2001:0db8::1]:80 [::]:*\ntcp LISTEN 0 1 2001:db8::1:80 :::*",
            "tcp LISTEN 0 1 *:80 0.0.0.0:*\ntcp LISTEN 0 1 0.0.0.0:80 *:*",
            "udp UNCONN 0 0 [::1%01]:53 *:*\nudp UNCONN 0 0 [::1%1]:53 *:*",
        ] {
            bad(input);
        }
        let entries = parse_listening_ports("tcp LISTEN 0 1 192.0.2.1:80 *:*\nudp UNCONN 0 0 192.0.2.1:80 *:*\ntcp LISTEN 0 1 192.0.2.2:80 *:*\ntcp LISTEN 0 1 [::1%eth0]:80 *:*\ntcp LISTEN 0 1 [::1%eth1]:80 *:*").unwrap();
        assert_eq!(entries.len(), 5);
    }

    struct Session {
        calls: AtomicUsize,
        output: &'static str,
        fail: bool,
        stall: bool,
        cancel: Option<CancellationToken>,
    }
    #[async_trait]
    impl RemoteSession for Session {
        async fn execute(
            &self,
            command: ReadOnlyCommand,
            _: CancellationToken,
        ) -> Result<String, AppError> {
            assert_eq!(command, ReadOnlyCommand::ListeningPorts);
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.stall {
                std::future::pending::<()>().await;
            }
            if self.fail {
                return Err(AppError::new(
                    ErrorCode::Connection,
                    "Synthetic transport failure",
                ));
            }
            if let Some(token) = &self.cancel {
                token.cancel();
            }
            Ok(self.output.into())
        }
        async fn disconnect(&self) -> Result<(), AppError> {
            panic!("No disconnect")
        }
        fn is_closed(&self) -> bool {
            false
        }
    }
    fn session() -> Session {
        Session {
            calls: AtomicUsize::new(0),
            output: "tcp LISTEN 0 1 192.0.2.1:22 *:*",
            fail: false,
            stall: false,
            cancel: None,
        }
    }
    #[tokio::test]
    async fn observes_exact_command_ownership_timestamp_and_empty_success() {
        let mut transport = session();
        let host = HostId::new();
        let owner = HostSessionId::new();
        let result = observe_listening_ports(&transport, CancellationToken::new(), host, owner)
            .await
            .unwrap();
        assert_eq!(result.host_id, host);
        assert_eq!(result.host_session_id, owner);
        assert!(!result.observed_at.is_empty());
        assert_eq!(result.entries.len(), 1);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        transport.output = "";
        assert!(
            observe_listening_ports(&transport, CancellationToken::new(), host, owner)
                .await
                .unwrap()
                .entries
                .is_empty()
        );
    }
    #[tokio::test]
    async fn transport_failure_cancellation_and_invalid_payload_never_become_empty_success() {
        let host = HostId::new();
        let owner = HostSessionId::new();
        let token = CancellationToken::new();
        token.cancel();
        let mut transport = session();
        assert_eq!(
            observe_listening_ports(&transport, token, host, owner)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
        transport.fail = true;
        assert_eq!(
            observe_listening_ports(&transport, CancellationToken::new(), host, owner)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Connection
        );
        transport.fail = false;
        transport.output = "invalid";
        assert_eq!(
            observe_listening_ports(&transport, CancellationToken::new(), host, owner)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Discovery
        );
        transport.output = "";
        let token = CancellationToken::new();
        transport.cancel = Some(token.clone());
        assert_eq!(
            observe_listening_ports(&transport, token, host, owner)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
    }
    #[tokio::test]
    async fn cancelled_inflight_transport_stops_without_retry() {
        let mut transport = session();
        transport.stall = true;
        let token = CancellationToken::new();
        let request = observe_listening_ports(
            &transport,
            token.clone(),
            HostId::new(),
            HostSessionId::new(),
        );
        tokio::pin!(request);
        tokio::select! { _ = &mut request => panic!("must stall"), _ = tokio::task::yield_now() => {} }
        token.cancel();
        assert_eq!(request.await.unwrap_err().code, ErrorCode::Cancelled);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }
}
