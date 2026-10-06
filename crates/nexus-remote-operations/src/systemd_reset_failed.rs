use crate::{NativeOperation, authority::sealed};
use nexus_model::{AppError, ErrorCode, OperationRisk};

const SUFFIX: &str = ".service";
const MAX_UNIT_BYTES: usize = 255;

/// A deliberately restricted, native-only systemd service target.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SystemdServiceUnitName(String);

impl SystemdServiceUnitName {
    pub fn parse(value: &str) -> Result<Self, AppError> {
        if value.is_empty()
            || value.len() > MAX_UNIT_BYTES
            || !value.is_ascii()
            || !value.ends_with(SUFFIX)
            || value.matches(SUFFIX).count() != 1
        {
            return Err(invalid());
        }
        let stem = &value[..value.len() - SUFFIX.len()];
        let mut parts = stem.split('@');
        let first = parts.next().unwrap_or_default();
        let second = parts.next();
        if parts.next().is_some()
            || !valid_part(first)
            || second.is_some_and(|part| !valid_part(part))
        {
            return Err(invalid());
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn valid_part(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'))
}

fn invalid() -> AppError {
    AppError::new(
        ErrorCode::Validation,
        "The observed service name is not supported for remote operations.",
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemdResetFailedPreconditions;

impl SystemdResetFailedPreconditions {
    pub fn matches(&self, load: &str, active: &str, sub: &str) -> bool {
        load == "loaded" && active == "failed" && sub == "failed"
    }
}

pub struct SystemdResetFailed;

impl sealed::Sealed for SystemdResetFailed {}

impl NativeOperation for SystemdResetFailed {
    type Target = SystemdServiceUnitName;
    type Preconditions = SystemdResetFailedPreconditions;
    type Payload = ();

    const RISK: OperationRisk = OperationRisk::Moderate;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operational_service_names_use_the_reviewed_subset() {
        for valid in [
            "ssh.service",
            "nginx.service",
            "my-worker.service",
            "worker@1.service",
            "worker@blue-1.service",
        ] {
            assert_eq!(
                SystemdServiceUnitName::parse(valid).unwrap().as_str(),
                valid
            );
        }
        let max = format!("{}.service", "a".repeat(255 - SUFFIX.len()));
        assert!(SystemdServiceUnitName::parse(&max).is_ok());
    }

    #[test]
    fn operational_service_names_reject_unsupported_or_ambiguous_values() {
        let oversized = format!("{}.service", "a".repeat(255 - SUFFIX.len() + 1));
        for invalid in [
            "",
            "--help.service",
            "foo@.service",
            "foo@@1.service",
            "foo;reboot.service",
            "foo$(id).service",
            "foo`id`.service",
            "foo bar.service",
            "../foo.service",
            "foo/service",
            "foo\nbar.service",
            "foo'.service",
            "foo\".service",
            "foo\\.service",
            "foo.service.service",
            "foo.timer",
            "f%6f.service",
            "f*.service",
            "é.service",
            &oversized,
        ] {
            assert!(
                SystemdServiceUnitName::parse(invalid).is_err(),
                "{invalid:?}"
            );
        }
    }
}
