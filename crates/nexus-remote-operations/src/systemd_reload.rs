use crate::{NativeOperation, SystemdServiceUnitName, authority::sealed};
use nexus_model::OperationRisk;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemdReloadPreconditions;

impl SystemdReloadPreconditions {
    pub fn matches(&self, load: &str, active: &str, sub: &str, can_reload: bool) -> bool {
        load == "loaded" && active == "active" && sub == "running" && can_reload
    }
}

pub struct SystemdReload;
impl sealed::Sealed for SystemdReload {}
impl NativeOperation for SystemdReload {
    type Target = SystemdServiceUnitName;
    type Preconditions = SystemdReloadPreconditions;
    type Payload = ();
    const RISK: OperationRisk = OperationRisk::High;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_reloadable_running_state_matches() {
        let p = SystemdReloadPreconditions;
        assert!(p.matches("loaded", "active", "running", true));
        for state in [
            ("loaded", "active", "running", false),
            ("loaded", "active", "exited", true),
            ("loaded", "inactive", "dead", true),
            ("loaded", "activating", "start", true),
            ("loaded", "deactivating", "stop", true),
            ("loaded", "failed", "failed", true),
            ("not-found", "inactive", "dead", true),
            ("masked", "active", "running", true),
        ] {
            assert!(!p.matches(state.0, state.1, state.2, state.3));
        }
    }
}
