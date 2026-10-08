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
            Self::SystemJournal => {
                "LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 journalctl --system --no-pager --quiet --boot=0 --reverse --lines=10 --output=json --output-fields=MESSAGE,PRIORITY,_SYSTEMD_UNIT,SYSLOG_IDENTIFIER"
            }
            Self::DockerContainers => {
                r#"LC_ALL=C docker --host unix:///var/run/docker.sock container ls --last 64 --no-trunc --format '{"id":{{json .ID}},"image":{{json .Image}},"name":{{json .Names}},"state":{{json .State}},"status":{{json .Status}},"ports":{{json .Ports}},"networks":{{json .Networks}}}'"#
            }
            Self::SystemServices => {
                "LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --all --type=service --property=Id --property=LoadState --property=ActiveState --property=SubState --property=CanReload --property=Description show"
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
            "LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --all --type=service --property=Id --property=LoadState --property=ActiveState --property=SubState --property=CanReload --property=Description show"
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
