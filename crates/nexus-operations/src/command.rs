use nexus_model::{Operation, OperationRisk};

/// Reviewed, fixed commands. Intentionally not deserializable through IPC.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadOnlyCommand {
    Hostname,
    OsRelease,
    Kernel,
    Architecture,
    Uptime,
    LoadAverage,
    Memory,
    RootFilesystem,
    CpuStat,
    NetworkDevices,
    NetworkAddresses,
    NetworkRoutesV4,
    NetworkRoutesV6,
    ListeningPorts,
    DnsConfiguration,
    SystemServices,
    SystemJournal,
    DockerContainers,
}

impl ReadOnlyCommand {
    /// Exact remote shell input. Remote values are never interpolated here.
    pub const fn command(self) -> &'static str {
        match self {
            Self::Hostname => "uname -n",
            Self::OsRelease => "cat /etc/os-release",
            Self::Kernel => "uname -r",
            Self::Architecture => "uname -m",
            Self::Uptime => "cat /proc/uptime",
            Self::LoadAverage => "cat /proc/loadavg",
            Self::Memory => "cat /proc/meminfo",
            Self::RootFilesystem => "LC_ALL=C df -Pk /",
            Self::CpuStat => "cat /proc/stat",
            Self::NetworkDevices => "cat /proc/net/dev",
            Self::NetworkAddresses => "LC_ALL=C ip -j address show",
            Self::NetworkRoutesV4 => "LC_ALL=C ip -j -4 route show table main",
            Self::NetworkRoutesV6 => "LC_ALL=C ip -j -6 route show table main",
            Self::ListeningPorts => "LC_ALL=C ss -H -lntu",
            Self::DnsConfiguration => "LC_ALL=C cat /etc/resolv.conf",
            Self::SystemJournal => {
                "LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 journalctl --system --no-pager --quiet --boot=0 --reverse --lines=10 --output=json --output-fields=MESSAGE,PRIORITY,_SYSTEMD_UNIT,SYSLOG_IDENTIFIER"
            }
            Self::DockerContainers => {
                r#"LC_ALL=C docker --host unix:///var/run/docker.sock container ls --last 64 --no-trunc --format '{"id":{{json .ID}},"image":{{json .Image}},"name":{{json .Names}},"state":{{json .State}},"status":{{json .Status}},"ports":{{json .Ports}},"networks":{{json .Networks}}}'"#
            }
            Self::SystemServices => {
                "LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --all --type=service --property=Id --property=LoadState --property=ActiveState --property=SubState --property=CanStart --property=CanReload --property=Description show"
            }
        }
    }

    pub const fn kind(self) -> &'static str {
        match self {
            Self::Hostname => "discovery.hostname",
            Self::OsRelease => "discovery.os_release",
            Self::Kernel => "discovery.kernel",
            Self::Architecture => "discovery.architecture",
            Self::Uptime => "discovery.uptime",
            Self::LoadAverage => "discovery.load",
            Self::Memory => "discovery.memory",
            Self::RootFilesystem => "discovery.root_filesystem",
            Self::CpuStat => "monitor.cpu",
            Self::NetworkDevices => "monitor.network",
            Self::NetworkAddresses => "network.list",
            Self::NetworkRoutesV4 => "network.routes_v4",
            Self::NetworkRoutesV6 => "network.routes_v6",
            Self::ListeningPorts => "network.listening_ports",
            Self::DnsConfiguration => "network.dns_configuration",
            Self::SystemServices => "services.list",
            Self::SystemJournal => "logs.list",
            Self::DockerContainers => "containers.list",
        }
    }

    pub fn operation(self) -> Operation {
        Operation {
            id: uuid::Uuid::new_v4().to_string(),
            kind: self.kind().into(),
            risk: OperationRisk::ReadOnly,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OperationEngine;
    use nexus_model::ErrorCode;

    #[test]
    fn docker_inventory_is_one_fixed_read_only_operation() {
        let command = ReadOnlyCommand::DockerContainers;
        assert_eq!(
            command.command(),
            r#"LC_ALL=C docker --host unix:///var/run/docker.sock container ls --last 64 --no-trunc --format '{"id":{{json .ID}},"image":{{json .Image}},"name":{{json .Names}},"state":{{json .State}},"status":{{json .Status}},"ports":{{json .Ports}},"networks":{{json .Networks}}}'"#
        );
        assert_eq!(command.kind(), "containers.list");
        assert_eq!(command.operation().risk, OperationRisk::ReadOnly);
        let engine = OperationEngine::default();
        let mut plan = engine.plan(command);
        plan.operation.kind = "logs.list".into();
        assert_eq!(engine.validate(&plan).unwrap_err().code, ErrorCode::Policy);
        plan.operation.kind = command.kind().into();
        for risk in [
            OperationRisk::Low,
            OperationRisk::Moderate,
            OperationRisk::High,
            OperationRisk::Destructive,
        ] {
            plan.operation.risk = risk;
            assert_eq!(engine.validate(&plan).unwrap_err().code, ErrorCode::Policy);
        }
    }

    #[test]
    fn journal_is_one_fixed_read_only_operation() {
        let command = ReadOnlyCommand::SystemJournal;
        assert_eq!(
            command.command(),
            "LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 journalctl --system --no-pager --quiet --boot=0 --reverse --lines=10 --output=json --output-fields=MESSAGE,PRIORITY,_SYSTEMD_UNIT,SYSLOG_IDENTIFIER"
        );
        assert_eq!(command.kind(), "logs.list");
        assert_eq!(command.operation().risk, OperationRisk::ReadOnly);
    }

    #[test]
    fn service_inventory_is_one_fixed_read_only_operation() {
        let command = ReadOnlyCommand::SystemServices;
        assert_eq!(
            command.command(),
            "LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --all --type=service --property=Id --property=LoadState --property=ActiveState --property=SubState --property=CanStart --property=CanReload --property=Description show"
        );
        assert_eq!(command.kind(), "services.list");
        assert_eq!(command.operation().risk, OperationRisk::ReadOnly);
        let engine = OperationEngine::default();
        let mut plan = engine.plan(command);
        plan.operation.kind = "discovery.hostname".into();
        assert_eq!(engine.validate(&plan).unwrap_err().code, ErrorCode::Policy);
        plan.operation.kind = command.kind().into();
        plan.operation.risk = OperationRisk::Low;
        assert_eq!(engine.validate(&plan).unwrap_err().code, ErrorCode::Policy);
    }

    #[test]
    fn network_inventory_is_one_fixed_read_only_operation() {
        let command = ReadOnlyCommand::NetworkAddresses;
        assert_eq!(command.command(), "LC_ALL=C ip -j address show");
        assert_eq!(command.kind(), "network.list");
        assert_eq!(command.operation().risk, OperationRisk::ReadOnly);
        let engine = OperationEngine::default();
        let mut plan = engine.plan(command);
        plan.operation.kind = "monitor.network".into();
        assert_eq!(engine.validate(&plan).unwrap_err().code, ErrorCode::Policy);
        plan.operation.kind = command.kind().into();
        plan.operation.risk = OperationRisk::Low;
        assert_eq!(engine.validate(&plan).unwrap_err().code, ErrorCode::Policy);
    }
}

#[cfg(test)]
mod route_tests {
    use super::*;
    use crate::OperationEngine;
    use nexus_model::ErrorCode;

    #[test]
    fn route_commands_have_exact_identity_readonly_policy_and_output_bounds() {
        let engine = OperationEngine::default();
        for (command, text, kind) in [
            (
                ReadOnlyCommand::NetworkRoutesV4,
                "LC_ALL=C ip -j -4 route show table main",
                "network.routes_v4",
            ),
            (
                ReadOnlyCommand::NetworkRoutesV6,
                "LC_ALL=C ip -j -6 route show table main",
                "network.routes_v6",
            ),
        ] {
            assert_eq!(command.command(), text);
            assert_eq!(command.kind(), kind);
            assert_eq!(command.operation().risk, OperationRisk::ReadOnly);
            let mut plan = engine.plan(command);
            plan.operation.kind = "network.list".into();
            assert_eq!(engine.validate(&plan).unwrap_err().code, ErrorCode::Policy);
            plan.operation.kind = kind.into();
            for risk in [
                OperationRisk::Low,
                OperationRisk::Moderate,
                OperationRisk::High,
                OperationRisk::Destructive,
            ] {
                plan.operation.risk = risk;
                assert_eq!(engine.validate(&plan).unwrap_err().code, ErrorCode::Policy);
            }
            let plan = engine.plan(command);
            assert!(engine.verify(&plan, &" ".repeat(64 * 1024)).is_ok());
            assert!(engine.verify(&plan, &" ".repeat(64 * 1024 + 1)).is_err());
            assert!(engine.verify(&plan, "\0").is_err());
        }
    }
}

#[cfg(test)]
mod listening_port_tests {
    use super::*;
    use crate::OperationEngine;
    #[test]
    fn exact_fixed_command_and_unchanged_readonly_policy_bounds() {
        let command = ReadOnlyCommand::ListeningPorts;
        assert_eq!(command.command(), "LC_ALL=C ss -H -lntu");
        assert_eq!(command.kind(), "network.listening_ports");
        assert_eq!(command.operation().risk, OperationRisk::ReadOnly);
        let engine = OperationEngine::default();
        let mut plan = engine.plan(command);
        plan.operation.kind = "network.list".into();
        assert!(engine.validate(&plan).is_err());
        plan.operation.kind = command.kind().into();
        for risk in [
            OperationRisk::Low,
            OperationRisk::Moderate,
            OperationRisk::High,
            OperationRisk::Destructive,
        ] {
            plan.operation.risk = risk;
            assert!(engine.validate(&plan).is_err());
        }
        let plan = engine.plan(command);
        assert!(engine.verify(&plan, &" ".repeat(64 * 1024)).is_ok());
        assert!(engine.verify(&plan, &" ".repeat(64 * 1024 + 1)).is_err());
        assert!(engine.verify(&plan, "\0").is_err());
    }
}

#[cfg(test)]
mod dns_configuration_tests {
    use super::*;
    use crate::OperationEngine;

    #[test]
    fn exact_fixed_command_and_unchanged_readonly_policy_bounds() {
        let command = ReadOnlyCommand::DnsConfiguration;
        assert_eq!(command.command(), "LC_ALL=C cat /etc/resolv.conf");
        assert_eq!(command.kind(), "network.dns_configuration");
        assert_eq!(command.operation().risk, OperationRisk::ReadOnly);
        let engine = OperationEngine::default();
        let mut plan = engine.plan(command);
        plan.operation.kind = "network.list".into();
        assert!(engine.validate(&plan).is_err());
        plan.operation.kind = command.kind().into();
        for risk in [
            OperationRisk::Low,
            OperationRisk::Moderate,
            OperationRisk::High,
            OperationRisk::Destructive,
        ] {
            plan.operation.risk = risk;
            assert!(engine.validate(&plan).is_err());
        }
        let plan = engine.plan(command);
        assert!(engine.verify(&plan, &" ".repeat(64 * 1024)).is_ok());
        assert!(engine.verify(&plan, &" ".repeat(64 * 1024 + 1)).is_err());
        assert!(engine.verify(&plan, "\0").is_err());
    }
}
