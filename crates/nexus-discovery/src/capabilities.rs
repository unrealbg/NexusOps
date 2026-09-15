use nexus_model::{AppError, DiscoverySnapshot, ErrorCode, HostCapability};
use std::collections::BTreeMap;

/// Stable capability identifiers permit later providers without changing domain enums.
#[derive(Default)]
pub struct CapabilityRegistry {
    values: BTreeMap<String, bool>,
}

impl CapabilityRegistry {
    pub fn register(&mut self, id: impl Into<String>, available: bool) -> Result<(), AppError> {
        let id = id.into();
        if id.is_empty()
            || id.len() > 64
            || !id.as_bytes()[0].is_ascii_lowercase()
            || !id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.'))
            || !id.as_bytes()[id.len() - 1].is_ascii_alphanumeric()
        {
            return Err(AppError::new(
                ErrorCode::Validation,
                "Capability identifiers must be lowercase names of at most 64 characters.",
            ));
        }
        self.values.insert(id, available);
        Ok(())
    }

    pub fn snapshot(&self) -> Vec<HostCapability> {
        self.values
            .iter()
            .map(|(id, available)| HostCapability {
                id: id.clone(),
                available: *available,
            })
            .collect()
    }
}

/// Report only baseline facts supported by successful Linux /proc probes.
pub fn capabilities(snapshot: &DiscoverySnapshot) -> Vec<HostCapability> {
    let procfs = snapshot.uptime_seconds.is_some()
        || snapshot.memory_total_bytes.is_some()
        || snapshot.load_one.is_some();
    if procfs {
        vec![
            HostCapability {
                id: "linux".into(),
                available: true,
            },
            HostCapability {
                id: "procfs".into(),
                available: true,
            },
        ]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_future_ids_and_updates_detection_without_duplicates() {
        let mut registry = CapabilityRegistry::default();
        registry
            .register("docker-compose", false)
            .expect("valid ID");
        registry.register("docker-compose", true).expect("update");
        registry.register("systemd", true).expect("valid ID");
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].id, "docker-compose");
        assert!(snapshot[0].available);
        for id in [
            "",
            "Docker",
            "a; rm",
            "-docker",
            "docker.",
            "docker/compose",
        ] {
            assert!(registry.register(id, true).is_err());
        }
    }
}
