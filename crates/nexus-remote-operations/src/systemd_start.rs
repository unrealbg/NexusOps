use crate::{NativeOperation, SystemdServiceUnitName, authority::sealed};
use nexus_model::OperationRisk;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemdStartPreconditions;

impl SystemdStartPreconditions {
    pub fn matches(&self, load: &str, active: &str, sub: &str, can_start: bool) -> bool {
        load == "loaded" && active == "inactive" && sub == "dead" && can_start
    }
}

pub struct SystemdStart;
impl sealed::Sealed for SystemdStart {}
impl NativeOperation for SystemdStart {
    type Target = SystemdServiceUnitName;
    type Preconditions = SystemdStartPreconditions;
    type Payload = ();
    const RISK: OperationRisk = OperationRisk::High;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_startable_inactive_dead_state_matches() {
        let p = SystemdStartPreconditions;
        assert!(p.matches("loaded", "inactive", "dead", true));
        for state in [
            ("loaded", "inactive", "dead", false),
            ("loaded", "inactive", "exited", true),
            ("loaded", "failed", "failed", true),
            ("loaded", "active", "running", true),
            ("loaded", "activating", "start", true),
            ("loaded", "deactivating", "stop-sigterm", true),
            ("not-found", "inactive", "dead", true),
        ] {
            assert!(!p.matches(state.0, state.1, state.2, state.3));
        }
    }
}
