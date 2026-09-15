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
