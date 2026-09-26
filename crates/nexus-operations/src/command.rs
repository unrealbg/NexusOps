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
    SystemServices,
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
            Self::SystemServices => {
                "LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-legend --plain --full --all --type=service --no-ask-password list-units"
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
            Self::SystemServices => "services.list",
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
    fn service_inventory_is_one_fixed_read_only_operation() {
        let command = ReadOnlyCommand::SystemServices;
        assert_eq!(
            command.command(),
            "LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-legend --plain --full --all --type=service --no-ask-password list-units"
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
}
