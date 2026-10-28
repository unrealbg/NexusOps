use crate::{HostId, HostSessionId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Validated IP address data from one fixed, read-only network observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum NetworkAddressFamily {
    Ipv4,
    Ipv6,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NetworkAddressEntry {
    pub family: NetworkAddressFamily,
    pub address: String,
    pub prefix_length: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterfaceEntry {
    pub ifindex: u32,
    pub name: String,
    pub oper_state: Option<String>,
    pub mtu: Option<u32>,
    pub addresses: Vec<NetworkAddressEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSnapshot {
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub observed_at: String,
    pub entries: Vec<NetworkInterfaceEntry>,
}

/// A validated simple unicast route in the Linux main table only.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NetworkRouteEntry {
    pub family: NetworkAddressFamily,
    // Canonical address/prefix, or `default` for a zero-length prefix.
    pub destination: String,
    pub gateway: Option<String>,
    pub interface: String,
    pub metric: Option<u32>,
    pub on_link: bool,
    pub link_down: bool,
    // IPv6 router preference, when reported; never inferred from a metric.
    pub preference: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NetworkRouteSnapshot {
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub observed_at: String,
    pub entries: Vec<NetworkRouteEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ListeningSocketProtocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ListeningSocketFamily {
    Ipv4,
    Ipv6,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ListeningSocketState {
    Listen,
    Unconn,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ListeningSocketEntry {
    pub protocol: ListeningSocketProtocol,
    pub family: ListeningSocketFamily,
    pub local_address: String,
    pub port: u16,
    pub state: ListeningSocketState,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ListeningPortSnapshot {
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub observed_at: String,
    pub entries: Vec<ListeningSocketEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum DnsSearchDirective {
    Search,
    Domain,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DnsNameserverEntry {
    pub address: String,
    pub family: NetworkAddressFamily,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DnsSortlistEntry {
    pub address: String,
    pub family: NetworkAddressFamily,
    pub netmask: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DnsConfigurationSnapshot {
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub observed_at: String,
    pub nameservers: Vec<DnsNameserverEntry>,
    pub search_directive: Option<DnsSearchDirective>,
    pub search_domains: Vec<String>,
    pub options: Vec<String>,
    pub sortlist: Vec<DnsSortlistEntry>,
}
