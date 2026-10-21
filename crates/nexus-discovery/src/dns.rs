use nexus_model::{
    AppError, DnsConfigurationSnapshot, DnsNameserverEntry, DnsSearchDirective, DnsSortlistEntry,
    ErrorCode, HostId, HostSessionId, NetworkAddressFamily,
};
use nexus_operations::{OperationEngine, ReadOnlyCommand, RemoteSession};
use std::net::IpAddr;
use tokio_util::sync::CancellationToken;

const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_LINES: usize = 256;
const MAX_LINE_BYTES: usize = 512;
const MAX_TOKENS_PER_LINE: usize = 32;
const MAX_NAMESERVERS: usize = 16;
const MAX_SEARCH_DOMAINS: usize = 31;
const MAX_OPTIONS: usize = 32;
const MAX_SORTLIST_ENTRIES: usize = 16;
const MAX_DNS_NAME_BYTES: usize = 253;
const MAX_OPTION_BYTES: usize = 64;
const MAX_SCOPE_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDnsConfiguration {
    pub nameservers: Vec<DnsNameserverEntry>,
    pub search_directive: Option<DnsSearchDirective>,
    pub search_domains: Vec<String>,
    pub options: Vec<String>,
    pub sortlist: Vec<DnsSortlistEntry>,
}

fn invalid() -> AppError {
    AppError::new(
        ErrorCode::Discovery,
        "The host returned an invalid or unsupported /etc/resolv.conf DNS configuration.",
    )
}

fn family(address: IpAddr) -> NetworkAddressFamily {
    match address {
        IpAddr::V4(_) => NetworkAddressFamily::Ipv4,
        IpAddr::V6(_) => NetworkAddressFamily::Ipv6,
    }
}

fn nameserver(value: &str) -> Result<DnsNameserverEntry, AppError> {
    let (raw, scope) = match value.split_once('%') {
        Some((raw, scope)) => {
            if scope.is_empty()
                || scope.len() > MAX_SCOPE_BYTES
                || !scope
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
            {
                return Err(invalid());
            }
            let scope = if scope.bytes().all(|byte| byte.is_ascii_digit()) {
                let index = scope.parse::<u32>().map_err(|_| invalid())?;
                if index == 0 {
                    return Err(invalid());
                }
                index.to_string()
            } else {
                scope.to_owned()
            };
            (raw, Some(scope))
        }
        None => (value, None),
    };
    let address: IpAddr = raw.parse().map_err(|_| invalid())?;
    if scope.is_some() && !address.is_ipv6() {
        return Err(invalid());
    }
    Ok(DnsNameserverEntry {
        address: match scope {
            Some(scope) => format!("{address}%{scope}"),
            None => address.to_string(),
        },
        family: family(address),
    })
}

fn valid_dns_suffix(value: &str) -> bool {
    if value == "." {
        return true;
    }
    if value.is_empty() || value.len() > MAX_DNS_NAME_BYTES || !value.is_ascii() {
        return false;
    }
    let name = value.strip_suffix('.').unwrap_or(value);
    if name.is_empty() {
        return false;
    }
    name.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && label.as_bytes()[0].is_ascii_alphanumeric()
            && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
            && (!label.to_ascii_lowercase().starts_with("xn--") || label.len() > 4)
    })
}

fn valid_option(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_OPTION_BYTES || !value.is_ascii() {
        return false;
    }
    let mut pieces = value.split(':');
    let name = pieces.next().unwrap_or_default();
    let argument = pieces.next();
    if pieces.next().is_some()
        || name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return false;
    }
    argument.is_none_or(|argument| {
        !argument.is_empty()
            && argument
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

fn contiguous_mask(mask: IpAddr) -> bool {
    let (bits, width) = match mask {
        IpAddr::V4(value) => (u128::from(u32::from(value)), 32),
        IpAddr::V6(value) => (u128::from(value), 128),
    };
    let relevant = if width == 32 { bits << 96 } else { bits };
    let inverted = !relevant;
    inverted == 0 || inverted & inverted.wrapping_add(1) == 0
}

fn parse_sortlist(value: &str) -> Result<DnsSortlistEntry, AppError> {
    if value.len() > 129 || value.matches('/').count() > 1 {
        return Err(invalid());
    }
    let (address, netmask) = value
        .split_once('/')
        .map_or((value, None), |(address, netmask)| (address, Some(netmask)));
    let address: IpAddr = address.parse().map_err(|_| invalid())?;
    let netmask = netmask
        .map(|value| value.parse::<IpAddr>().map_err(|_| invalid()))
        .transpose()?;
    if netmask.is_some_and(|mask| family(mask) != family(address) || !contiguous_mask(mask)) {
        return Err(invalid());
    }
    Ok(DnsSortlistEntry {
        address: address.to_string(),
        family: family(address),
        netmask: netmask.map(|mask| mask.to_string()),
    })
}

fn validate_file_shape(output: &str) -> Result<(), AppError> {
    if output.len() > MAX_OUTPUT_BYTES {
        return Err(invalid());
    }
    let line_count = output.bytes().filter(|byte| *byte == b'\n').count()
        + usize::from(!output.is_empty() && !output.ends_with('\n'));
    if line_count > MAX_LINES {
        return Err(invalid());
    }
    let bytes = output.as_bytes();
    for (index, byte) in bytes.iter().copied().enumerate() {
        if byte == b'\r' {
            if bytes.get(index + 1) != Some(&b'\n') {
                return Err(invalid());
            }
        } else if byte != b'\n' && byte != b'\t' && !(b' '..=b'~').contains(&byte) {
            return Err(invalid());
        }
    }
    Ok(())
}

/// Parse only bounded, selected `/etc/resolv.conf` directives. Unsupported active
/// configuration rejects the whole snapshot; comments and raw text are discarded.
pub fn parse_dns_configuration(output: &str) -> Result<ParsedDnsConfiguration, AppError> {
    validate_file_shape(output)?;
    let mut parsed = ParsedDnsConfiguration {
        nameservers: Vec::new(),
        search_directive: None,
        search_domains: Vec::new(),
        options: Vec::new(),
        sortlist: Vec::new(),
    };
    for line in output.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.len() > MAX_LINE_BYTES {
            return Err(invalid());
        }
        let comment = [line.find('#'), line.find(';')]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(line.len());
        let active = line[..comment].trim();
        if active.is_empty() {
            continue;
        }
        let fields: Vec<_> = active.split_ascii_whitespace().collect();
        if fields.len() > MAX_TOKENS_PER_LINE {
            return Err(invalid());
        }
        match fields.as_slice() {
            ["nameserver", value] => {
                if parsed.nameservers.len() == MAX_NAMESERVERS {
                    return Err(invalid());
                }
                parsed.nameservers.push(nameserver(value)?);
            }
            ["search", domains @ ..] if !domains.is_empty() => {
                if domains.len() > MAX_SEARCH_DOMAINS
                    || domains.iter().any(|domain| !valid_dns_suffix(domain))
                {
                    return Err(invalid());
                }
                parsed.search_directive = Some(DnsSearchDirective::Search);
                parsed.search_domains = domains.iter().map(|domain| (*domain).to_owned()).collect();
            }
            ["domain", domain] if valid_dns_suffix(domain) => {
                parsed.search_directive = Some(DnsSearchDirective::Domain);
                parsed.search_domains = vec![(*domain).to_owned()];
            }
            ["options", options @ ..] if !options.is_empty() => {
                if parsed.options.len() + options.len() > MAX_OPTIONS
                    || options.iter().any(|option| !valid_option(option))
                {
                    return Err(invalid());
                }
                parsed
                    .options
                    .extend(options.iter().map(|option| (*option).to_owned()));
            }
            ["sortlist", entries @ ..] if !entries.is_empty() => {
                if parsed.sortlist.len() + entries.len() > MAX_SORTLIST_ENTRIES {
                    return Err(invalid());
                }
                for entry in entries {
                    parsed.sortlist.push(parse_sortlist(entry)?);
                }
            }
            _ => return Err(invalid()),
        }
    }
    Ok(parsed)
}

pub async fn observe_dns_configuration(
    session: &dyn RemoteSession,
    cancellation: CancellationToken,
    host_id: HostId,
    host_session_id: HostSessionId,
) -> Result<DnsConfigurationSnapshot, AppError> {
    let engine = OperationEngine::default();
    let output = engine
        .execute(
            session,
            &engine.plan(ReadOnlyCommand::DnsConfiguration),
            cancellation.clone(),
        )
        .await?;
    let parsed = parse_dns_configuration(&output)?;
    if cancellation.is_cancelled() {
        return Err(AppError::new(
            ErrorCode::Cancelled,
            "The DNS configuration observation was cancelled.",
        ));
    }
    Ok(DnsConfigurationSnapshot {
        host_id,
        host_session_id,
        observed_at: chrono::Utc::now().to_rfc3339(),
        nameservers: parsed.nameservers,
        search_directive: parsed.search_directive,
        search_domains: parsed.search_domains,
        options: parsed.options,
        sortlist: parsed.sortlist,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn bad(value: &str) {
        let error = parse_dns_configuration(value).unwrap_err();
        assert_eq!(error.code, ErrorCode::Discovery);
        assert_eq!(
            error.message,
            "The host returned an invalid or unsupported /etc/resolv.conf DNS configuration."
        );
    }

    #[test]
    fn accepts_empty_comments_crlf_stubs_ipv4_ipv6_and_discards_comment_text() {
        assert!(parse_dns_configuration("").unwrap().nameservers.is_empty());
        assert!(
            parse_dns_configuration(" \t\r\n")
                .unwrap()
                .nameservers
                .is_empty()
        );
        let parsed = parse_dns_configuration(
            "# generated by systemd-resolved SECRET-INTERNAL\r\n; another secret\r\nnameserver 127.0.0.53 # local stub\r\nnameserver 2001:0db8::53\r\n",
        )
        .unwrap();
        assert_eq!(
            parsed.nameservers,
            vec![
                DnsNameserverEntry {
                    address: "127.0.0.53".into(),
                    family: NetworkAddressFamily::Ipv4,
                },
                DnsNameserverEntry {
                    address: "2001:db8::53".into(),
                    family: NetworkAddressFamily::Ipv6,
                },
            ]
        );
        assert!(!format!("{parsed:?}").contains("SECRET-INTERNAL"));
    }

    #[test]
    fn accepts_canonical_ipv4_ipv6_and_strict_scoped_ipv6_nameservers_in_order() {
        let parsed = parse_dns_configuration(
            "nameserver 192.0.2.53\nnameserver fe80::1%eth0\nnameserver fe80::abcd%enp1s0\nnameserver FE80:0:0:0:0:0:0:ABCD%0002\nnameserver 2001:0DB8:0:0::53\nnameserver fe80::2%veth_0.1-2\n",
        )
        .unwrap();
        assert_eq!(
            parsed.nameservers,
            vec![
                DnsNameserverEntry {
                    address: "192.0.2.53".into(),
                    family: NetworkAddressFamily::Ipv4,
                },
                DnsNameserverEntry {
                    address: "fe80::1%eth0".into(),
                    family: NetworkAddressFamily::Ipv6,
                },
                DnsNameserverEntry {
                    address: "fe80::abcd%enp1s0".into(),
                    family: NetworkAddressFamily::Ipv6,
                },
                DnsNameserverEntry {
                    address: "fe80::abcd%2".into(),
                    family: NetworkAddressFamily::Ipv6,
                },
                DnsNameserverEntry {
                    address: "2001:db8::53".into(),
                    family: NetworkAddressFamily::Ipv6,
                },
                DnsNameserverEntry {
                    address: "fe80::2%veth_0.1-2".into(),
                    family: NetworkAddressFamily::Ipv6,
                },
            ]
        );
    }

    #[test]
    fn last_search_or_domain_wins_while_observed_options_sortlist_and_nameserver_order_remain() {
        let parsed = parse_dns_configuration(
            "nameserver 2001:db8::2\nnameserver 192.0.2.53\nsearch EXAMPLE.com xn--bcher-kva.example.\ndomain final.example\noptions ndots:5 rotate\noptions single-request-reopen\nsortlist 192.0.2.0/255.255.255.0 2001:db8::/ffff:ffff:ffff:ffff::\n",
        )
        .unwrap();
        assert_eq!(parsed.nameservers[0].address, "2001:db8::2");
        assert_eq!(parsed.nameservers[1].address, "192.0.2.53");
        assert_eq!(parsed.search_directive, Some(DnsSearchDirective::Domain));
        assert_eq!(parsed.search_domains, ["final.example"]);
        assert_eq!(
            parsed.options,
            ["ndots:5", "rotate", "single-request-reopen"]
        );
        assert_eq!(parsed.sortlist.len(), 2);
        assert_eq!(parsed.sortlist[0].netmask.as_deref(), Some("255.255.255.0"));
        assert_eq!(
            parsed.sortlist[1].netmask.as_deref(),
            Some("ffff:ffff:ffff:ffff::")
        );

        let zero_masks = parse_dns_configuration("sortlist 0.0.0.0/0.0.0.0 ::/::\n").unwrap();
        assert_eq!(zero_masks.sortlist[0].netmask.as_deref(), Some("0.0.0.0"));
        assert_eq!(zero_masks.sortlist[1].netmask.as_deref(), Some("::"));

        let parsed =
            parse_dns_configuration("domain old.example\nsearch one.example two.example\n")
                .unwrap();
        assert_eq!(parsed.search_directive, Some(DnsSearchDirective::Search));
        assert_eq!(parsed.search_domains, ["one.example", "two.example"]);
    }

    #[test]
    fn repeated_observations_are_preserved_without_interpreting_effects() {
        let parsed = parse_dns_configuration(
            "nameserver 192.0.2.1\nnameserver 192.0.2.1\noptions rotate\noptions rotate\nsortlist 192.0.2.0\nsortlist 192.0.2.0\n",
        )
        .unwrap();
        assert_eq!(parsed.nameservers.len(), 2);
        assert_eq!(parsed.options, ["rotate", "rotate"]);
        assert_eq!(parsed.sortlist.len(), 2);
    }

    #[test]
    fn malformed_active_configuration_and_unsafe_text_fail_closed() {
        for input in [
            "nameserver",
            "nameserver 999.1.1.1",
            "nameserver 192.0.2.1 extra",
            "search",
            "search -bad.example",
            "search bad_.example",
            "domain one.example two.example",
            "domain xn--",
            "options",
            "options ndots::5",
            "sortlist 192.0.2.0/255.0.255.0",
            "sortlist 192.0.2.0/ffff::",
            "lookup bind",
            "nameserver 192.0.2.1\rbad",
            "nameserver 192.0.2.1\u{1b}[31m",
            "nameserver 192.0.2.1\u{7f}",
            "nameserver 192.0.2.1\nnonascii café.example",
        ] {
            bad(input);
        }
    }

    #[test]
    fn rejects_invalid_unsupported_and_unsafe_nameserver_scopes() {
        for value in [
            "192.0.2.1%eth0",
            "192.0.2.1%2",
            "fe80::1%",
            "fe80::1%0",
            "fe80::1%0000",
            "fe80::1%4294967296",
            "fe80::1%eth0%2",
            "fe80::1%eth/0",
            "fe80::1%eth:0",
            "fe80::gggg%eth0",
            "fe80::1%eth\u{1b}[31m",
            "fe80::1%eth\u{7f}",
            "fe80::1%eth\u{202e}0",
        ] {
            bad(&format!("nameserver {value}\n"));
        }
        bad(&format!(
            "nameserver fe80::1%{}\n",
            "a".repeat(MAX_SCOPE_BYTES + 1)
        ));
        bad("nameserver 192.0.2.53\nnameserver fe80::1%0\n");
    }

    #[test]
    fn byte_line_token_and_collection_limits_are_enforced() {
        bad(&"x".repeat(MAX_OUTPUT_BYTES + 1));
        bad(&format!("#{}", "x".repeat(MAX_LINE_BYTES)));
        bad(&"#\n".repeat(MAX_LINES + 1));
        bad(&format!(
            "options {}\n",
            vec!["x"; MAX_TOKENS_PER_LINE].join(" ")
        ));
        bad(&format!(
            "{}\n",
            (0..=MAX_NAMESERVERS)
                .map(|_| "nameserver 192.0.2.1")
                .collect::<Vec<_>>()
                .join("\n")
        ));
        bad(&format!("domain {}.example", "a".repeat(64)));
    }

    struct Session {
        calls: AtomicUsize,
        result: Result<String, AppError>,
    }

    #[async_trait]
    impl RemoteSession for Session {
        async fn execute(
            &self,
            command: ReadOnlyCommand,
            _: CancellationToken,
        ) -> Result<String, AppError> {
            assert_eq!(command, ReadOnlyCommand::DnsConfiguration);
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.result.clone()
        }
        async fn disconnect(&self) -> Result<(), AppError> {
            Ok(())
        }
        fn is_closed(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn observation_executes_once_propagates_transport_failure_and_honors_cancellation() {
        let host = HostId::new();
        let owner = HostSessionId::new();
        let session = Session {
            calls: AtomicUsize::new(0),
            result: Ok("nameserver 127.0.0.53\n".into()),
        };
        let snapshot = observe_dns_configuration(&session, CancellationToken::new(), host, owner)
            .await
            .unwrap();
        assert_eq!(snapshot.host_id, host);
        assert_eq!(snapshot.host_session_id, owner);
        assert_eq!(snapshot.nameservers[0].address, "127.0.0.53");
        assert_eq!(session.calls.load(Ordering::SeqCst), 1);

        let session = Session {
            calls: AtomicUsize::new(0),
            result: Err(AppError::new(ErrorCode::Connection, "synthetic failure")),
        };
        assert_eq!(
            observe_dns_configuration(&session, CancellationToken::new(), host, owner)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Connection
        );
        assert_eq!(session.calls.load(Ordering::SeqCst), 1);

        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let session = Session {
            calls: AtomicUsize::new(0),
            result: Ok(String::new()),
        };
        assert_eq!(
            observe_dns_configuration(&session, cancellation, host, owner)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
        assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    }
}
