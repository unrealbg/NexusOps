use crate::parsers;
use nexus_model::{AppError, DiscoverySnapshot};
use nexus_operations::ReadOnlyCommand;

/// An extension point for discovery providers with individually testable parsers.
/// A probe may choose only a reviewed command from the operation allowlist.
pub trait HostProbe: Send + Sync {
    fn id(&self) -> &'static str;
    fn command(&self) -> ReadOnlyCommand;
    fn parse(&self, output: &str, snapshot: &mut DiscoverySnapshot) -> Result<(), AppError>;
}

struct LinuxProbe(ReadOnlyCommand);

impl HostProbe for LinuxProbe {
    fn id(&self) -> &'static str {
        self.0.kind()
    }
    fn command(&self) -> ReadOnlyCommand {
        self.0
    }

    fn parse(&self, output: &str, snapshot: &mut DiscoverySnapshot) -> Result<(), AppError> {
        match self.0 {
            ReadOnlyCommand::Hostname => snapshot.hostname = Some(parsers::scalar(output)?),
            ReadOnlyCommand::OsRelease => {
                let (name, version) = parsers::os_release(output)?;
                snapshot.os = Some(name);
                snapshot.os_version = version;
            }
            ReadOnlyCommand::Kernel => snapshot.kernel = Some(parsers::scalar(output)?),
            ReadOnlyCommand::Architecture => snapshot.architecture = Some(parsers::scalar(output)?),
            ReadOnlyCommand::Uptime => {
                snapshot.uptime_seconds = Some(parsers::first_number(output)?)
            }
            ReadOnlyCommand::LoadAverage => {
                snapshot.load_one = Some(parsers::first_number(output)?)
            }
            ReadOnlyCommand::Memory => {
                let (total, used) = parsers::memory(output)?;
                snapshot.memory_total_bytes = Some(total);
                snapshot.memory_used_bytes = Some(used);
            }
            ReadOnlyCommand::RootFilesystem => {
                let (total, used) = parsers::root_filesystem(output)?;
                snapshot.root_total_bytes = Some(total);
                snapshot.root_used_bytes = Some(used);
            }
        }
        Ok(())
    }
}

static BUILTIN_PROBES: [LinuxProbe; 8] = [
    LinuxProbe(ReadOnlyCommand::Hostname),
    LinuxProbe(ReadOnlyCommand::OsRelease),
    LinuxProbe(ReadOnlyCommand::Kernel),
    LinuxProbe(ReadOnlyCommand::Architecture),
    LinuxProbe(ReadOnlyCommand::Uptime),
    LinuxProbe(ReadOnlyCommand::LoadAverage),
    LinuxProbe(ReadOnlyCommand::Memory),
    LinuxProbe(ReadOnlyCommand::RootFilesystem),
];

pub fn builtin_probes() -> impl Iterator<Item = &'static dyn HostProbe> {
    BUILTIN_PROBES.iter().map(|probe| probe as &dyn HostProbe)
}
