use nexus_model::{
    AppError, ErrorCode, HostId, HostSessionId, NetworkAddressEntry, NetworkAddressFamily,
    NetworkInterfaceEntry, NetworkSnapshot,
};
use nexus_operations::{OperationEngine, ReadOnlyCommand, RemoteSession};
use serde::{Deserialize, Deserializer};
use std::{collections::HashSet, net::IpAddr};
use tokio_util::sync::CancellationToken;

pub const MAX_INTERFACES: usize = 128;
pub const MAX_ADDRESSES: usize = 512;
pub const MAX_IFNAME_BYTES: usize = 64;
pub const MAX_STATE_BYTES: usize = 32;
pub const MAX_RAW_IP_BYTES: usize = 64;

fn invalid() -> AppError {
    AppError::new(
        ErrorCode::Discovery,
        "The host returned an invalid network inventory.",
    )
}

// Derived deserialization rejects duplicate selected fields. Unrelated iproute2
// fields, including link-layer addresses, are ignored and never published.
#[derive(Deserialize)]
struct RawInterface {
    ifindex: u32,
    ifname: String,
    addr_info: Vec<RawAddress>,
    mtu: Option<u32>,
    operstate: Option<String>,
}

#[derive(Deserialize)]
struct RawAddress {
    #[serde(default, deserialize_with = "present")]
    family: Option<String>,
    #[serde(default, deserialize_with = "present")]
    family_index: Option<u32>,
    local: Option<String>,
    prefixlen: Option<u16>,
}

// Distinguish an absent representation from a present null value. Both
// representations present, even if one is null, must fail closed.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn unsafe_display(character: char) -> bool {
    character.is_control()
        || character.is_whitespace()
        || matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        || matches!(character, '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}')
}

fn display_token(value: &str, max_bytes: usize) -> Result<(), AppError> {
    if value.is_empty() || value.len() > max_bytes || value.chars().any(unsafe_display) {
        return Err(invalid());
    }
    Ok(())
}

fn parse_address(raw: RawAddress) -> Result<Option<NetworkAddressEntry>, AppError> {
    let family = match (raw.family.as_deref(), raw.family_index) {
        (Some("inet"), None) => NetworkAddressFamily::Ipv4,
        (Some("inet6"), None) => NetworkAddressFamily::Ipv6,
        (Some(_), None) | (None, Some(_)) => return Ok(None),
        _ => return Err(invalid()),
    };
    let local = raw.local.ok_or_else(invalid)?;
    let prefix = raw.prefixlen.ok_or_else(invalid)?;
    if local.is_empty() || local.len() > MAX_RAW_IP_BYTES {
        return Err(invalid());
    }
    let address: IpAddr = local.parse().map_err(|_| invalid())?;
    match (family, address) {
        (NetworkAddressFamily::Ipv4, IpAddr::V4(_)) if prefix <= 32 => {}
        (NetworkAddressFamily::Ipv6, IpAddr::V6(_)) if prefix <= 128 => {}
        _ => return Err(invalid()),
    }
    Ok(Some(NetworkAddressEntry {
        family,
        address: address.to_string(),
        prefix_length: u8::try_from(prefix).map_err(|_| invalid())?,
    }))
}

/// Parse only selected fields from one `ip -j address show` response.
/// Remote values are never included in errors or retained after validation.
pub fn parse_network(output: &str) -> Result<Vec<NetworkInterfaceEntry>, AppError> {
    let raw: Vec<RawInterface> = serde_json::from_str(output).map_err(|_| invalid())?;
    if raw.len() > MAX_INTERFACES {
        return Err(invalid());
    }
    let mut indexes = HashSet::new();
    let mut names = HashSet::new();
    let mut raw_address_count = 0usize;
    let mut entries = Vec::with_capacity(raw.len());
    for interface in raw {
        display_token(&interface.ifname, MAX_IFNAME_BYTES)?;
        if interface.ifindex == 0
            || !indexes.insert(interface.ifindex)
            || !names.insert(interface.ifname.clone())
        {
            return Err(invalid());
        }
        if let Some(state) = &interface.operstate {
            display_token(state, MAX_STATE_BYTES)?;
        }
        raw_address_count = raw_address_count
            .checked_add(interface.addr_info.len())
            .ok_or_else(invalid)?;
        if raw_address_count > MAX_ADDRESSES {
            return Err(invalid());
        }
        let mut addresses = Vec::new();
        let mut seen = HashSet::new();
        for raw_address in interface.addr_info {
            if let Some(address) = parse_address(raw_address)? {
                if !seen.insert((
                    address.family,
                    address.address.clone(),
                    address.prefix_length,
                )) {
                    return Err(invalid());
                }
                addresses.push(address);
            }
        }
        entries.push(NetworkInterfaceEntry {
            ifindex: interface.ifindex,
            name: interface.ifname,
            oper_state: interface.operstate,
            mtu: interface.mtu,
            addresses,
        });
    }
    Ok(entries)
}

pub async fn observe_network(
    session: &dyn RemoteSession,
    cancellation: CancellationToken,
    host_id: HostId,
    host_session_id: HostSessionId,
) -> Result<NetworkSnapshot, AppError> {
    let engine = OperationEngine::default();
    let plan = engine.plan(ReadOnlyCommand::NetworkAddresses);
    let output = engine.execute(session, &plan, cancellation).await?;
    let entries = parse_network(&output)?;
    Ok(NetworkSnapshot {
        host_id,
        host_session_id,
        observed_at: chrono::Utc::now().to_rfc3339(),
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(index: u32, name: &str, addresses: &str) -> String {
        format!(
            r#"{{"ifindex":{index},"ifname":"{name}","mtu":1500,"operstate":"UP","addr_info":[{addresses}]}}"#
        )
    }

    fn bad(input: &str) {
        let error = parse_network(input).expect_err("invalid network output");
        assert_eq!(error.code, ErrorCode::Discovery);
        assert_eq!(
            error.message,
            "The host returned an invalid network inventory."
        );
        assert!(!error.message.contains("192.0.2"));
    }

    #[test]
    fn accepts_realistic_selected_fields_and_canonicalizes_ip_without_link_metadata() {
        let input = r#"[{"ifindex":1,"ifname":"lo","flags":["LOOPBACK"],"address":"00:00:00:00:00:00","mtu":65536,"operstate":"UNKNOWN","addr_info":[{"family":"inet","local":"127.0.0.1","prefixlen":8,"valid_life_time":4294967295},{"family":"inet6","local":"0:0:0:0:0:0:0:1","prefixlen":128}]},{"ifindex":2,"ifname":"eth0","mtu":1500,"operstate":"FUTURE_STATE","addr_info":[{"family":"inet","local":"192.0.2.10","prefixlen":24},{"family":"inet6","local":"2001:0db8:0:0:0:0:0:10","prefixlen":64}]},{"ifindex":3,"ifname":"empty0","addr_info":[]}]"#;
        let entries = parse_network(input).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].addresses[0].address, "127.0.0.1");
        assert_eq!(entries[0].addresses[1].address, "::1");
        assert_eq!(entries[1].addresses[1].address, "2001:db8::10");
        assert_eq!(entries[1].oper_state.as_deref(), Some("FUTURE_STATE"));
        assert_eq!(entries[2].oper_state, None);
        assert_eq!(entries[2].mtu, None);
        assert!(entries[2].addresses.is_empty());
        assert!(
            !serde_json::to_string(&entries)
                .unwrap()
                .contains("00:00:00")
        );
    }

    #[test]
    fn ignores_unsupported_family_but_counts_it_toward_raw_limit() {
        let unsupported = r#"{"family":"link","local":"00:11:22:33:44:55","prefixlen":0}"#;
        let input = format!("[{}]", iface(1, "eth0", unsupported));
        assert!(parse_network(&input).unwrap()[0].addresses.is_empty());
        let many = vec![unsupported; MAX_ADDRESSES + 1].join(",");
        bad(&format!("[{}]", iface(1, "eth0", &many)));
    }

    #[test]
    fn preserves_inet_addresses_and_omits_numeric_unknown_family() {
        let input = r#"[{"ifindex":2,"ifname":"eth0","addr_info":[{"family":"inet","local":"192.0.2.10","prefixlen":24},{"family":"inet6","local":"2001:0db8::10","prefixlen":64},{"family_index":45,"local":"8","prefixlen":0,"valid_life_time":4294967295}]}]"#;
        let entries = parse_network(input).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].addresses.len(), 2);
        assert_eq!(entries[0].addresses[0].address, "192.0.2.10");
        assert_eq!(entries[0].addresses[1].address, "2001:db8::10");
        assert!(
            !serde_json::to_string(&entries)
                .unwrap()
                .contains("family_index")
        );
    }

    #[test]
    fn numeric_unknown_families_count_toward_raw_address_limit() {
        let unsupported = r#"{"family_index":45,"local":"8","prefixlen":0}"#;
        let at_limit = vec![unsupported; MAX_ADDRESSES].join(",");
        assert!(
            parse_network(&format!("[{}]", iface(1, "eth0", &at_limit))).unwrap()[0]
                .addresses
                .is_empty()
        );
        let over_limit = vec![unsupported; MAX_ADDRESSES + 1].join(",");
        bad(&format!("[{}]", iface(1, "eth0", &over_limit)));
    }

    #[test]
    fn rejects_ambiguous_duplicate_or_missing_address_family_representation() {
        for address in [
            r#"{"family_index":45,"family_index":46}"#,
            r#"{"family":"inet","family_index":45,"local":"192.0.2.1","prefixlen":24}"#,
            r#"{"family_index":45,"family":"inet","local":"192.0.2.1","prefixlen":24}"#,
            r#"{"local":"192.0.2.1","prefixlen":24}"#,
            r#"{"family":null,"family_index":45}"#,
            r#"{"family_index":null}"#,
            r#"{"family_index":"45"}"#,
        ] {
            bad(&format!("[{}]", iface(1, "eth0", address)));
        }
    }

    #[test]
    fn rejects_malformed_selected_fields_and_duplicate_json_keys() {
        for input in [
            "{}",
            "[] trailing",
            "[{}]",
            "[{\"ifindex\":1,\"ifname\":\"eth0\"}]",
            "[{\"ifindex\":1,\"ifindex\":2,\"ifname\":\"eth0\",\"addr_info\":[]}]",
            "[{\"ifindex\":1,\"ifname\":\"eth0\",\"addr_info\":[],\"operstate\":\"UP\",\"operstate\":\"DOWN\"}]",
            "[{\"ifindex\":1.5,\"ifname\":\"eth0\",\"addr_info\":[]}]",
            "[{\"ifindex\":1,\"ifname\":\"eth0\",\"mtu\":\"large\",\"addr_info\":[]}]",
            "[{\"ifindex\":1,\"ifname\":\"eth0\",\"addr_info\":[{\"family\":\"inet\",\"family\":\"inet6\",\"local\":\"192.0.2.1\",\"prefixlen\":24}]}]",
        ] {
            bad(input);
        }
    }

    #[test]
    fn rejects_invalid_interfaces_and_all_bounds() {
        bad(&format!("[{}]", iface(0, "eth0", "")));
        bad(&format!(
            "[{},{}]",
            iface(1, "eth0", ""),
            iface(1, "eth1", "")
        ));
        bad(&format!(
            "[{},{}]",
            iface(1, "eth0", ""),
            iface(2, "eth0", "")
        ));
        bad(&format!("[{}]", iface(1, "", "")));
        bad(&format!(
            "[{}]",
            iface(1, &"a".repeat(MAX_IFNAME_BYTES + 1), "")
        ));
        bad(&format!("[{}]", iface(1, "bad name", "")));
        bad(&format!("[{}]", iface(1, "bad\\nname", "")));
        for ch in ["\\u202e", "\\u2066", "\\u200b", "\\u0001"] {
            bad(&format!("[{}]", iface(1, &format!("bad{ch}name"), "")));
        }
        for state in [
            "",
            "bad state",
            "bad\\u202e",
            &"X".repeat(MAX_STATE_BYTES + 1),
        ] {
            let input = iface(1, "eth0", "").replace("\"UP\"", &format!("\"{state}\""));
            bad(&format!("[{input}]"));
        }
        let many = (1..=MAX_INTERFACES + 1)
            .map(|i| iface(i as u32, &format!("eth{i}"), ""))
            .collect::<Vec<_>>()
            .join(",");
        bad(&format!("[{many}]"));
    }

    #[test]
    fn rejects_invalid_ip_family_prefix_duplicate_and_raw_length() {
        let address = |family: &str, local: &str, prefix: u16| {
            format!(r#"{{"family":"{family}","local":"{local}","prefixlen":{prefix}}}"#)
        };
        for entry in [
            address("inet", "2001:db8::1", 24),
            address("inet6", "192.0.2.1", 64),
            address("inet", "999.1.1.1", 24),
            address("inet6", "bad::ip", 64),
            address("inet", "192.0.2.1", 33),
            address("inet6", "::1", 129),
            address("inet", &"1".repeat(MAX_RAW_IP_BYTES + 1), 24),
            r#"{"family":"inet","prefixlen":24}"#.into(),
            r#"{"family":"inet","local":"192.0.2.1"}"#.into(),
        ] {
            bad(&format!("[{}]", iface(1, "eth0", &entry)));
        }
        let duplicate = [
            address("inet6", "2001:0db8::1", 64),
            address("inet6", "2001:db8::1", 64),
        ]
        .join(",");
        bad(&format!("[{}]", iface(1, "eth0", &duplicate)));
    }
}
