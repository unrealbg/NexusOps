use crate::parsers;
use nexus_model::{AppError, ErrorCode, HostId, HostMonitorSample, HostSessionId};
use nexus_operations::{OperationEngine, ReadOnlyCommand, RemoteSession};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const MAX_CPU_FIELDS: usize = 16;
const MAX_INTERFACES: usize = 128;
const MAX_INTERFACE_NAME: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuCounters {
    fields: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkCounters {
    interfaces: BTreeMap<String, (u64, u64)>,
}

#[derive(Debug, Clone)]
pub struct MonitorReading {
    pub observed_at: String,
    pub cpu: Option<CpuCounters>,
    pub memory: Option<(u64, u64)>,
    pub swap: Option<(u64, u64)>,
    pub root: Option<(u64, u64)>,
    pub network: Option<NetworkCounters>,
    pub warnings: Vec<AppError>,
}

#[derive(Debug, Clone)]
pub struct MonitorBaseline {
    cpu: Option<CpuCounters>,
    network: Option<NetworkCounters>,
}

fn invalid() -> AppError {
    AppError::new(
        ErrorCode::Discovery,
        "The host returned an unsupported monitoring value.",
    )
}

pub fn parse_cpu(output: &str) -> Result<CpuCounters, AppError> {
    let row = output
        .lines()
        .find(|line| line.starts_with("cpu "))
        .ok_or_else(invalid)?;
    let values = row
        .split_whitespace()
        .skip(1)
        .map(|part| part.parse::<u64>().map_err(|_| invalid()))
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() < 4 || values.len() > MAX_CPU_FIELDS {
        return Err(invalid());
    }
    // Validate that the counters relevant to the formula can be summed.
    values.iter().take(8).try_fold(0_u64, |sum, value| {
        sum.checked_add(*value).ok_or_else(invalid)
    })?;
    Ok(CpuCounters { fields: values })
}

pub fn parse_swap(output: &str) -> Result<(u64, u64), AppError> {
    let mut total = None;
    let mut free = None;
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key != "SwapTotal" && key != "SwapFree" {
            continue;
        }
        let mut parts = value.split_whitespace();
        let bytes = parts
            .next()
            .ok_or_else(invalid)?
            .parse::<u64>()
            .map_err(|_| invalid())?
            .checked_mul(1024)
            .ok_or_else(invalid)?;
        if parts.next() != Some("kB") || parts.next().is_some() {
            return Err(invalid());
        }
        let target = if key == "SwapTotal" {
            &mut total
        } else {
            &mut free
        };
        if target.replace(bytes).is_some() {
            return Err(invalid());
        }
    }
    let (total, free) = (total.ok_or_else(invalid)?, free.ok_or_else(invalid)?);
    Ok((total, total.checked_sub(free).ok_or_else(invalid)?))
}

pub fn parse_network(output: &str) -> Result<NetworkCounters, AppError> {
    let mut interfaces = BTreeMap::new();
    for line in output
        .lines()
        .skip(2)
        .filter(|line| !line.trim().is_empty())
    {
        if interfaces.len() >= MAX_INTERFACES {
            return Err(invalid());
        }
        let (name, counters) = line.split_once(':').ok_or_else(invalid)?;
        let name = name.trim();
        if name.is_empty()
            || name.len() > MAX_INTERFACE_NAME
            || name.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(invalid());
        }
        let values = counters
            .split_whitespace()
            .map(|part| part.parse::<u64>().map_err(|_| invalid()))
            .collect::<Result<Vec<_>, _>>()?;
        if values.len() != 16
            || interfaces
                .insert(name.to_owned(), (values[0], values[8]))
                .is_some()
        {
            return Err(invalid());
        }
    }
    if interfaces.is_empty() {
        return Err(invalid());
    }
    aggregate_network(&interfaces)?;
    Ok(NetworkCounters { interfaces })
}

fn aggregate_network(values: &BTreeMap<String, (u64, u64)>) -> Result<(u64, u64), AppError> {
    values
        .iter()
        .filter(|(name, _)| name.as_str() != "lo")
        .try_fold((0_u64, 0_u64), |(rx, tx), (_, (next_rx, next_tx))| {
            Ok((
                rx.checked_add(*next_rx).ok_or_else(invalid)?,
                tx.checked_add(*next_tx).ok_or_else(invalid)?,
            ))
        })
}

fn cpu_percent(current: &CpuCounters, previous: &CpuCounters) -> Option<f64> {
    if current.fields.len() != previous.fields.len() {
        return None;
    }
    let current = &current.fields[..current.fields.len().min(8)];
    let previous = &previous.fields[..previous.fields.len().min(8)];
    let deltas = current
        .iter()
        .zip(previous)
        .map(|(a, b)| a.checked_sub(*b))
        .collect::<Option<Vec<_>>>()?;
    let total = deltas
        .iter()
        .try_fold(0_u64, |sum, value| sum.checked_add(*value))?;
    if total == 0 {
        return None;
    }
    let idle = deltas[3].checked_add(*deltas.get(4).unwrap_or(&0))?;
    let busy = total.checked_sub(idle)?;
    let value = busy as f64 / total as f64 * 100.0;
    if value.is_finite() && (-1e-9..=100.0 + 1e-9).contains(&value) {
        Some(value.clamp(0.0, 100.0))
    } else {
        None
    }
}

fn network_rates(
    current: &NetworkCounters,
    previous: &NetworkCounters,
    elapsed: Duration,
) -> Option<(f64, f64)> {
    if elapsed.is_zero() {
        return None;
    }
    let current_names = current.interfaces.keys().collect::<BTreeSet<_>>();
    let previous_names = previous.interfaces.keys().collect::<BTreeSet<_>>();
    if current_names != previous_names {
        return None;
    }
    let mut rx_delta = 0_u64;
    let mut tx_delta = 0_u64;
    for (name, (rx, tx)) in &current.interfaces {
        let (old_rx, old_tx) = previous.interfaces.get(name)?;
        let (drx, dtx) = (rx.checked_sub(*old_rx)?, tx.checked_sub(*old_tx)?);
        if name != "lo" {
            rx_delta = rx_delta.checked_add(drx)?;
            tx_delta = tx_delta.checked_add(dtx)?;
        }
    }
    let seconds = elapsed.as_secs_f64();
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    let rates = (rx_delta as f64 / seconds, tx_delta as f64 / seconds);
    (rates.0.is_finite() && rates.1.is_finite()).then_some(rates)
}

pub async fn observe_monitor(
    session: &dyn RemoteSession,
    cancellation: CancellationToken,
) -> Result<MonitorReading, AppError> {
    let engine = OperationEngine::default();
    let mut reading = MonitorReading {
        observed_at: chrono::Utc::now().to_rfc3339(),
        cpu: None,
        memory: None,
        swap: None,
        root: None,
        network: None,
        warnings: Vec::new(),
    };
    for command in [
        ReadOnlyCommand::CpuStat,
        ReadOnlyCommand::Memory,
        ReadOnlyCommand::RootFilesystem,
        ReadOnlyCommand::NetworkDevices,
    ] {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let result = engine
            .execute(session, &engine.plan(command), cancellation.clone())
            .await;
        match result {
            Ok(output) => {
                let parsed = match command {
                    ReadOnlyCommand::CpuStat => {
                        parse_cpu(&output).map(|value| reading.cpu = Some(value))
                    }
                    ReadOnlyCommand::Memory => {
                        parsers::memory(&output).map(|value| reading.memory = Some(value))
                    }
                    ReadOnlyCommand::RootFilesystem => {
                        parsers::root_filesystem(&output).map(|value| reading.root = Some(value))
                    }
                    ReadOnlyCommand::NetworkDevices => {
                        parse_network(&output).map(|value| reading.network = Some(value))
                    }
                    _ => unreachable!(),
                };
                if command == ReadOnlyCommand::Memory
                    && let Err(error) = parse_swap(&output).map(|value| reading.swap = Some(value))
                {
                    reading.warnings.push(metric_warning("swap", error.code));
                }
                if let Err(error) = parsed {
                    reading
                        .warnings
                        .push(metric_warning(command.kind(), error.code));
                }
            }
            Err(error) if error.code == ErrorCode::Cancelled || cancellation.is_cancelled() => {
                return Err(cancelled());
            }
            Err(error) if error.code == ErrorCode::Connection => return Err(error),
            Err(error) => {
                if command == ReadOnlyCommand::Memory {
                    reading.warnings.push(metric_warning("swap", error.code));
                }
                reading
                    .warnings
                    .push(metric_warning(command.kind(), error.code));
            }
        }
    }
    reading.observed_at = chrono::Utc::now().to_rfc3339();
    Ok(reading)
}

fn metric_warning(metric: &str, code: ErrorCode) -> AppError {
    let name = match metric {
        "monitor.cpu" => "CPU",
        "discovery.memory" => "memory",
        "swap" => "swap",
        "discovery.root_filesystem" => "root filesystem",
        "monitor.network" => "network",
        _ => "monitoring",
    };
    AppError::new(code, format!("{name} monitoring is unavailable."))
}

fn cancelled() -> AppError {
    AppError::new(ErrorCode::Cancelled, "Host monitoring was cancelled.")
}

pub fn derive_sample(
    host_id: HostId,
    host_session_id: HostSessionId,
    reading: MonitorReading,
    previous: Option<&MonitorBaseline>,
    elapsed: Option<Duration>,
) -> (HostMonitorSample, MonitorBaseline) {
    let cpu_usage_percent = previous
        .and_then(|old| reading.cpu.as_ref().zip(old.cpu.as_ref()))
        .and_then(|(a, b)| cpu_percent(a, b));
    let (network_rx_bytes_per_second, network_tx_bytes_per_second) = previous
        .and_then(|old| reading.network.as_ref().zip(old.network.as_ref()))
        .and_then(|(a, b)| elapsed.and_then(|duration| network_rates(a, b, duration)))
        .map_or((None, None), |(rx, tx)| (Some(rx), Some(tx)));
    let baseline = MonitorBaseline {
        cpu: reading.cpu.clone(),
        network: reading.network.clone(),
    };
    let sample = HostMonitorSample {
        host_id,
        host_session_id,
        observed_at: reading.observed_at,
        cpu_usage_percent,
        memory_total_bytes: reading.memory.map(|value| value.0),
        memory_used_bytes: reading.memory.map(|value| value.1),
        swap_total_bytes: reading.swap.map(|value| value.0),
        swap_used_bytes: reading.swap.map(|value| value.1),
        root_total_bytes: reading.root.map(|value| value.0),
        root_used_bytes: reading.root.map(|value| value.1),
        network_rx_bytes_per_second,
        network_tx_bytes_per_second,
        warnings: reading.warnings,
    };
    (sample, baseline)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn reading(cpu: &str, network: &str) -> MonitorReading {
        MonitorReading {
            observed_at: "2026-09-25T00:00:00Z".into(),
            cpu: Some(parse_cpu(cpu).unwrap()),
            memory: Some((100, 25)),
            swap: Some((0, 0)),
            root: Some((100, 20)),
            network: Some(parse_network(network).unwrap()),
            warnings: vec![],
        }
    }
    const NET_A: &str = "Inter-| Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n lo: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0\n eth0: 1000 0 0 0 0 0 0 0 2000 0 0 0 0 0 0 0\n";
    const NET_B: &str = "Inter-| Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n lo: 30 0 0 0 0 0 0 0 40 0 0 0 0 0 0 0\n eth0: 1500 0 0 0 0 0 0 0 3000 0 0 0 0 0 0 0\n";

    #[test]
    fn parses_cpu_and_derives_known_delta() {
        let first = reading("cpu 100 0 50 850 10 0 0 0 1 2\ncpu0 1\n", NET_A);
        let (sample, baseline) =
            derive_sample(HostId::new(), HostSessionId::new(), first, None, None);
        assert!(sample.cpu_usage_percent.is_none());
        let second = reading("cpu 140 0 60 900 10 0 0 0 3 4\n", NET_B);
        let (sample, _) = derive_sample(
            sample.host_id,
            sample.host_session_id,
            second,
            Some(&baseline),
            Some(Duration::from_secs(5)),
        );
        assert_eq!(sample.cpu_usage_percent, Some(50.0));
        assert_eq!(sample.network_rx_bytes_per_second, Some(100.0));
        assert_eq!(sample.network_tx_bytes_per_second, Some(200.0));
    }

    #[test]
    fn rejects_missing_malformed_overflow_and_resets() {
        assert!(parse_cpu("cpu0 1 2 3 4\n").is_err());
        assert!(parse_cpu("cpu 1 x 3 4\n").is_err());
        assert!(parse_cpu(&format!("cpu {} 1 1 1\n", u64::MAX)).is_err());
        let old = MonitorBaseline {
            cpu: Some(parse_cpu("cpu 10 1 1 10\n").unwrap()),
            network: Some(parse_network(NET_B).unwrap()),
        };
        let current = reading("cpu 9 1 1 10\n", NET_A);
        let (sample, _) = derive_sample(
            HostId::new(),
            HostSessionId::new(),
            current,
            Some(&old),
            Some(Duration::from_secs(1)),
        );
        assert!(sample.cpu_usage_percent.is_none());
        assert!(sample.network_rx_bytes_per_second.is_none());
    }

    #[test]
    fn parses_swap_zero_active_and_rejects_bad_values() {
        assert_eq!(
            parse_swap("SwapTotal: 0 kB\nSwapFree: 0 kB\n").unwrap(),
            (0, 0)
        );
        assert_eq!(
            parse_swap("SwapTotal: 100 kB\nSwapFree: 40 kB\n").unwrap(),
            (102400, 61440)
        );
        for value in [
            "SwapTotal: 1 kB\n",
            "SwapTotal: 1 kB\nSwapTotal: 1 kB\nSwapFree: 0 kB\n",
            "SwapTotal: 1 kB\nSwapFree: 2 kB\n",
            "SwapTotal: 18446744073709551615 kB\nSwapFree: 0 kB\n",
        ] {
            assert!(parse_swap(value).is_err());
        }
    }

    #[test]
    fn parses_network_excludes_loopback_and_enforces_bounds() {
        let parsed = parse_network(NET_A).unwrap();
        assert_eq!(aggregate_network(&parsed.interfaces).unwrap(), (1000, 2000));
        let multiple = parse_network("a\nb\n lo: 9 0 0 0 0 0 0 0 9 0 0 0 0 0 0 0\n eth0: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0\n\twlan0:\t30 0 0 0 0 0 0 0\t40 0 0 0 0 0 0 0\n").unwrap();
        assert_eq!(aggregate_network(&multiple.interfaces).unwrap(), (40, 60));
        assert!(parse_network("a\nb\n eth0: 1 2\n").is_err());
        assert!(parse_network("a\nb\n eth0: 1 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0\n eth0: 2 0 0 0 0 0 0 0 2 0 0 0 0 0 0 0\n").is_err());
        let too_many = (0..129)
            .map(|i| format!("eth{i}: 1 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(parse_network(&format!("a\nb\n{too_many}\n")).is_err());
        assert!(parse_network(&format!("a\nb\neth0: {} 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0\neth1: 1 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0\n", u64::MAX)).is_err());
        assert!(network_rates(&parsed, &parsed, Duration::ZERO).is_none());
    }
}
