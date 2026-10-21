use nexus_model::{
    AppError, ErrorCode, HostId, HostSessionId, NetworkAddressFamily, NetworkRouteEntry,
    NetworkRouteSnapshot,
};
use nexus_operations::{OperationEngine, ReadOnlyCommand, RemoteSession};
use serde::{Deserialize, Deserializer};
use std::{collections::HashSet, net::IpAddr};
use tokio_util::sync::CancellationToken;

const MAX_ROUTES: usize = 128;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

fn invalid() -> AppError {
    AppError::new(
        ErrorCode::Discovery,
        "The host returned an invalid or unsupported main-table route inventory.",
    )
}

// Missing optional fields are allowed; present nulls and duplicate selected
// fields are not. Unknown fields fail closed: they may change forwarding
// semantics (multipath, nhid, encap, via, src, tos, route metrics, etc.).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRoute {
    dst: String,
    dev: String,
    #[serde(default, deserialize_with = "present")]
    gateway: Option<String>,
    #[serde(default, deserialize_with = "present")]
    metric: Option<u32>,
    #[serde(default, deserialize_with = "present", rename = "type")]
    kind: Option<String>,
    #[serde(default, deserialize_with = "present")]
    protocol: Option<Protocol>,
    #[serde(default, deserialize_with = "present")]
    scope: Option<String>,
    #[serde(default, deserialize_with = "present")]
    prefsrc: Option<String>,
    #[serde(default, deserialize_with = "present")]
    table: Option<Table>,
    #[serde(default, deserialize_with = "present")]
    flags: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    pref: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Protocol {
    Name(String),
    Number(u32),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Table {
    Name(String),
    Number(u32),
}

fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn token(value: &str, bound: usize) -> bool {
    !value.is_empty()
        && value.len() <= bound
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'@')
        })
}

fn address(value: &str, family: NetworkAddressFamily) -> Result<IpAddr, AppError> {
    if value.len() > 64 {
        return Err(invalid());
    }
    let ip: IpAddr = value.parse().map_err(|_| invalid())?;
    match (family, ip) {
        (NetworkAddressFamily::Ipv4, IpAddr::V4(_))
        | (NetworkAddressFamily::Ipv6, IpAddr::V6(_)) => Ok(ip),
        _ => Err(invalid()),
    }
}

fn destination(value: &str, family: NetworkAddressFamily) -> Result<String, AppError> {
    if value == "default" {
        return Ok(value.into());
    }
    if value.len() > 68 {
        return Err(invalid());
    }
    let max = if family == NetworkAddressFamily::Ipv4 {
        32
    } else {
        128
    };
    let (raw, prefix) = match value.split_once('/') {
        Some((raw, prefix)) => {
            if prefix.is_empty() || !prefix.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(invalid());
            }
            (raw, prefix.parse::<u32>().map_err(|_| invalid())?)
        }
        None => (value, max),
    };
    if prefix > max {
        return Err(invalid());
    }
    let ip = address(raw, family)?;
    // Non-canonical network addresses would create ambiguous route identities.
    let bits = match ip {
        IpAddr::V4(ip) => u128::from(u32::from(ip)),
        IpAddr::V6(ip) => u128::from(ip),
    };
    let host_mask = if max - prefix == 128 {
        u128::MAX
    } else {
        (1u128 << (max - prefix)) - 1
    };
    if bits & host_mask != 0 {
        return Err(invalid());
    }
    Ok(if prefix == 0 {
        "default".into()
    } else {
        format!("{ip}/{prefix}")
    })
}

/// Only complete, bounded inventories of supported simple unicast routes pass.
/// A rejected row rejects the entire observation; no row is silently omitted.
pub fn parse_routes(
    output: &str,
    family: NetworkAddressFamily,
) -> Result<Vec<NetworkRouteEntry>, AppError> {
    if output.len() > MAX_OUTPUT_BYTES {
        return Err(invalid());
    }
    let raw: Vec<RawRoute> = serde_json::from_str(output).map_err(|_| invalid())?;
    if raw.len() > MAX_ROUTES {
        return Err(invalid());
    }
    let mut entries = Vec::with_capacity(raw.len());
    let mut identities = HashSet::new();
    for route in raw {
        if !token(&route.dev, 64) || route.kind.as_deref().is_some_and(|kind| kind != "unicast") {
            return Err(invalid());
        }
        if let Some(protocol) = route.protocol {
            match protocol {
                Protocol::Name(name) if token(&name, 32) => {}
                Protocol::Number(number) if number <= 255 => {}
                _ => return Err(invalid()),
            }
        }
        if route
            .scope
            .as_deref()
            .is_some_and(|scope| !matches!(scope, "global" | "universe" | "link" | "host"))
        {
            return Err(invalid());
        }
        if let Some(source) = route.prefsrc {
            address(&source, family)?;
        }
        if let Some(table) = route.table {
            match table {
                Table::Name(name) if name == "main" => {}
                Table::Number(254) => {}
                _ => return Err(invalid()),
            }
        }
        let mut flags = HashSet::new();
        for flag in route.flags.unwrap_or_default() {
            if !matches!(flag.as_str(), "onlink" | "linkdown") || !flags.insert(flag) {
                return Err(invalid());
            }
        }
        if route.pref.as_deref().is_some_and(|pref| {
            family != NetworkAddressFamily::Ipv6 || !matches!(pref, "low" | "medium" | "high")
        }) {
            return Err(invalid());
        }
        let entry = NetworkRouteEntry {
            family,
            destination: destination(&route.dst, family)?,
            gateway: route
                .gateway
                .map(|gateway| address(&gateway, family).map(|ip| ip.to_string()))
                .transpose()?,
            interface: route.dev,
            metric: route.metric,
            on_link: flags.contains("onlink"),
            link_down: flags.contains("linkdown"),
            preference: route.pref,
        };
        // Flags describe an existing route, not another route identity.
        if !identities.insert((
            entry.destination.clone(),
            entry.gateway.clone(),
            entry.interface.clone(),
            entry.metric,
        )) {
            return Err(invalid());
        }
        entries.push(entry);
    }
    Ok(entries)
}

pub async fn observe_routes(
    session: &dyn RemoteSession,
    cancellation: CancellationToken,
    host_id: HostId,
    host_session_id: HostSessionId,
) -> Result<NetworkRouteSnapshot, AppError> {
    let engine = OperationEngine::default();
    let mut entries = Vec::new();
    for (command, family) in [
        (ReadOnlyCommand::NetworkRoutesV4, NetworkAddressFamily::Ipv4),
        (ReadOnlyCommand::NetworkRoutesV6, NetworkAddressFamily::Ipv6),
    ] {
        let output = engine
            .execute(session, &engine.plan(command), cancellation.clone())
            .await?;
        entries.extend(parse_routes(&output, family)?);
        if entries.len() > MAX_ROUTES {
            return Err(invalid());
        }
    }
    if cancellation.is_cancelled() {
        return Err(AppError::new(
            ErrorCode::Cancelled,
            "The route observation was cancelled.",
        ));
    }
    Ok(NetworkRouteSnapshot {
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
    use std::sync::Mutex;

    const V4: NetworkAddressFamily = NetworkAddressFamily::Ipv4;
    const V6: NetworkAddressFamily = NetworkAddressFamily::Ipv6;
    const DEFAULT: &str = r#"[{"dst":"default","gateway":"192.0.2.1","dev":"eth0"}]"#;

    #[test]
    fn preserves_multiple_defaults_direct_host_routes_and_optional_metrics() {
        let entries = parse_routes(r#"[
          {"dst":"default","gateway":"192.0.2.1","dev":"eth0","protocol":"dhcp","metric":100,"flags":["onlink"]},
          {"dst":"0.0.0.0/0","gateway":"198.51.100.1","dev":"eth1","metric":200},
          {"dst":"192.0.2.0/24","dev":"eth0","protocol":"kernel","scope":"link","prefsrc":"192.0.2.10","table":254,"flags":[]},
          {"dst":"203.0.113.1","dev":"eth2","metric":0,"type":"unicast","table":"main","flags":["linkdown"]}
        ]"#, V4).unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[1].destination, "default");
        assert!(entries[0].on_link);
        assert_eq!(entries[2].gateway, None);
        assert_eq!(entries[2].metric, None);
        assert_eq!(entries[3].metric, Some(0));
        assert_eq!(entries[3].destination, "203.0.113.1/32");
        assert!(entries[3].link_down);
        let v6 = parse_routes(r#"[{"dst":"default","gateway":"2001:0db8::1","dev":"eth0","protocol":"ra","metric":1024,"pref":"high"},{"dst":"2001:0db8::/64","dev":"eth0"},{"dst":"2001:db8::42","dev":"eth1"}]"#, V6).unwrap();
        assert_eq!(v6[0].gateway.as_deref(), Some("2001:db8::1"));
        assert_eq!(v6[0].preference.as_deref(), Some("high"));
        assert_eq!(v6[1].destination, "2001:db8::/64");
        assert_eq!(v6[2].destination, "2001:db8::42/128");
        assert!(parse_routes("[]", V4).unwrap().is_empty());
    }

    fn bad(value: &str, family: NetworkAddressFamily) {
        let error = parse_routes(value, family).unwrap_err();
        assert_eq!(error.code, ErrorCode::Discovery);
        assert_eq!(
            error.message,
            "The host returned an invalid or unsupported main-table route inventory."
        );
    }

    #[test]
    fn malformed_missing_duplicate_and_null_fields_fail_closed() {
        for input in [
            "",
            "{}",
            "[",
            "[] trailing",
            "[{}]",
            r#"[{"dst":"default","dev":"eth0","dst":"192.0.2.0/24"}]"#,
            r#"[{"dst":"default","dev":"eth0","dev":"eth1"}]"#,
            r#"[{"dst":"default","dev":"eth0","metric":1,"metric":2}]"#,
        ] {
            bad(input, V4);
        }
        for field in [
            "gateway", "metric", "type", "protocol", "scope", "prefsrc", "table", "flags", "pref",
        ] {
            bad(
                &format!(r#"[{{"dst":"default","dev":"eth0","{field}":null}}]"#),
                V4,
            );
            bad(
                &format!(r#"[{{"dst":"default","dev":"eth0","{field}":null,"{field}":null}}]"#),
                V4,
            );
        }
    }

    #[test]
    fn invalid_addresses_prefixes_metrics_and_display_tokens_are_rejected() {
        for (family, dst) in [
            (V4, "999.1.1.1"),
            (V4, "2001:db8::/32"),
            (V6, "192.0.2.0/24"),
            (V4, "192.0.2.0/33"),
            (V6, "2001:db8::/129"),
            (V4, "192.0.2.1/24"),
            (V4, "192.0.2.0/-1"),
            (V4, "192.0.2.0/+24"),
            (V4, "192.0.2.0/24/1"),
            (V6, "2001:db8::1/64"),
        ] {
            bad(&format!(r#"[{{"dst":"{dst}","dev":"eth0"}}]"#), family);
        }
        for field in [
            r#""gateway":"2001:db8::1""#,
            r#""gateway":"bad""#,
            r#""metric":-1"#,
            r#""metric":1.5"#,
            r#""metric":4294967296"#,
            r#""metric":"1""#,
            r#""flags":["onlink","onlink"]"#,
            r#""flags":["mystery"]"#,
            r#""protocol":256"#,
            r#""protocol":"bad name""#,
            r#""scope":"bad""#,
            r#""prefsrc":"bad""#,
            r#""table":100"#,
            r#""pref":"high""#,
        ] {
            bad(
                &format!(r#"[{{"dst":"default","dev":"eth0",{field}}}]"#),
                V4,
            );
        }
        for dev in [
            "",
            "bad name",
            "bad\\nname",
            "bad\\u202ename",
            "<img>",
            &"a".repeat(65),
        ] {
            bad(&format!(r#"[{{"dst":"default","dev":"{dev}"}}]"#), V4);
        }
    }

    #[test]
    fn unsupported_forwarding_is_never_silently_flattened() {
        for field in [
            r#""nexthops":[]"#,
            r#""nhid":7"#,
            r#""encap":{}"#,
            r#""via":{}"#,
            r#""src":"2001:db8::/64""#,
            r#""tos":16"#,
            r#""mtu":1400"#,
            r#""metrics":{}"#,
            r#""unknown_future_field":1"#,
            r#""type":"blackhole""#,
            r#""type":"unreachable""#,
            r#""type":"local""#,
        ] {
            bad(
                &format!(r#"[{{"dst":"default","dev":"eth0",{field}}}]"#),
                V4,
            );
        }
    }

    fn inventory(count: usize) -> String {
        format!(
            "[{}]",
            (0..count)
                .map(|i| format!(r#"{{"dst":"192.0.2.{i}","dev":"eth0"}}"#))
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    #[test]
    fn enforces_route_output_and_canonical_identity_bounds() {
        assert_eq!(parse_routes(&inventory(128), V4).unwrap().len(), 128);
        bad(&inventory(129), V4);
        bad(
            &format!(
                "[{},{}]",
                &DEFAULT[1..DEFAULT.len() - 1],
                &DEFAULT[1..DEFAULT.len() - 1]
            ),
            V4,
        );
        bad(
            r#"[{"dst":"default","dev":"eth0"},{"dst":"0.0.0.0/0","dev":"eth0","flags":["linkdown"]}]"#,
            V4,
        );
        assert!(parse_routes(&format!("[]{}", " ".repeat(MAX_OUTPUT_BYTES - 2)), V4).is_ok());
        bad(&format!("[]{}", " ".repeat(MAX_OUTPUT_BYTES - 1)), V4);
    }

    struct Session {
        replies: Mutex<std::collections::VecDeque<Result<String, AppError>>>,
        calls: Mutex<Vec<ReadOnlyCommand>>,
        cancel_after: Option<(usize, CancellationToken)>,
    }
    #[async_trait]
    impl RemoteSession for Session {
        async fn execute(
            &self,
            command: ReadOnlyCommand,
            _: CancellationToken,
        ) -> Result<String, AppError> {
            self.calls.lock().unwrap().push(command);
            if let Some((count, token)) = &self.cancel_after
                && self.calls.lock().unwrap().len() == *count
            {
                token.cancel();
            }
            self.replies.lock().unwrap().pop_front().unwrap()
        }
        async fn disconnect(&self) -> Result<(), AppError> {
            panic!("observation cannot disconnect")
        }
        fn is_closed(&self) -> bool {
            false
        }
    }
    fn session(replies: Vec<Result<String, AppError>>) -> Session {
        Session {
            replies: Mutex::new(replies.into()),
            calls: Mutex::new(vec![]),
            cancel_after: None,
        }
    }
    async fn observe(
        session: &Session,
        cancellation: CancellationToken,
    ) -> Result<NetworkRouteSnapshot, AppError> {
        observe_routes(session, cancellation, HostId::new(), HostSessionId::new()).await
    }

    #[tokio::test]
    async fn one_owned_snapshot_requires_both_fixed_commands_and_combined_limit() {
        let s = session(vec![Ok(DEFAULT.into()), Ok("[]".into())]);
        let host = HostId::new();
        let owner = HostSessionId::new();
        let result = observe_routes(&s, CancellationToken::new(), host, owner)
            .await
            .unwrap();
        assert_eq!(result.host_id, host);
        assert_eq!(result.host_session_id, owner);
        assert!(!result.observed_at.is_empty());
        assert_eq!(result.entries.len(), 1);
        assert_eq!(
            *s.calls.lock().unwrap(),
            vec![
                ReadOnlyCommand::NetworkRoutesV4,
                ReadOnlyCommand::NetworkRoutesV6
            ]
        );
        let s = session(vec![
            Ok(inventory(127)),
            Ok(r#"[{"dst":"default","dev":"eth0"}]"#.into()),
        ]);
        assert_eq!(
            observe(&s, CancellationToken::new())
                .await
                .unwrap()
                .entries
                .len(),
            128
        );
        let s = session(vec![
            Ok(inventory(128)),
            Ok(r#"[{"dst":"default","dev":"eth0"}]"#.into()),
        ]);
        assert_eq!(
            observe(&s, CancellationToken::new())
                .await
                .unwrap_err()
                .code,
            ErrorCode::Discovery
        );
    }

    #[tokio::test]
    async fn no_partial_success_on_either_failure_invalid_second_family_or_cancellation() {
        for replies in [
            vec![Err(AppError::new(
                ErrorCode::Connection,
                "Synthetic failure",
            ))],
            vec![
                Ok(DEFAULT.into()),
                Err(AppError::new(ErrorCode::Timeout, "Synthetic failure")),
            ],
            vec![Ok(DEFAULT.into()), Ok("[{}]".into())],
        ] {
            assert!(
                observe(&session(replies), CancellationToken::new())
                    .await
                    .is_err()
            );
        }
        let token = CancellationToken::new();
        token.cancel();
        let s = session(vec![]);
        assert_eq!(
            observe(&s, token).await.unwrap_err().code,
            ErrorCode::Cancelled
        );
        assert!(s.calls.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn cancellation_at_either_family_boundary_rejects_complete_or_partial_payloads() {
        for count in [1, 2] {
            let token = CancellationToken::new();
            let mut s = session(vec![Ok(DEFAULT.into()), Ok("[]".into())]);
            s.cancel_after = Some((count, token.clone()));
            assert_eq!(
                observe(&s, token).await.unwrap_err().code,
                ErrorCode::Cancelled
            );
            assert_eq!(s.calls.lock().unwrap().len(), count);
        }
    }
}
