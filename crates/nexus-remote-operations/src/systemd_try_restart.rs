use crate::{NativeOperation, SystemdServiceUnitName, authority::sealed};
use nexus_model::OperationRisk;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemdTryRestartPreconditions;

impl SystemdTryRestartPreconditions {
    pub fn matches(&self, load: &str, active: &str, sub: &str) -> bool {
        load == "loaded" && active == "active" && sub == "running"
    }
}

pub struct SystemdTryRestart;

impl sealed::Sealed for SystemdTryRestart {}

impl NativeOperation for SystemdTryRestart {
    type Target = SystemdServiceUnitName;
    type Preconditions = SystemdTryRestartPreconditions;
    type Payload = ();

    const RISK: OperationRisk = OperationRisk::High;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_loaded_active_running_state_matches() {
        let preconditions = SystemdTryRestartPreconditions;
        assert!(preconditions.matches("loaded", "active", "running"));
        for state in [
            ("loaded", "active", "exited"),
            ("loaded", "activating", "start"),
            ("loaded", "deactivating", "stop"),
            ("loaded", "failed", "failed"),
            ("loaded", "inactive", "dead"),
            ("masked", "active", "running"),
        ] {
            assert!(!preconditions.matches(state.0, state.1, state.2));
        }
    }
}
