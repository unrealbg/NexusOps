use crate::RemoteSession;
use nexus_model::{AppError, ErrorCode};
use std::{collections::HashSet, time::Duration};
use tokio_util::sync::CancellationToken;

pub const SYSTEMD_STOP_IMPACT_PROPERTIES: [&str; 37] = [
    "Id",
    "Names",
    "Following",
    "LoadState",
    "ActiveState",
    "SubState",
    "CanStop",
    "RefuseManualStop",
    "Job",
    "NeedDaemonReload",
    "StopWhenUnneeded",
    "Requires",
    "RequiredBy",
    "Requisite",
    "RequisiteOf",
    "Wants",
    "WantedBy",
    "BindsTo",
    "BoundBy",
    "PartOf",
    "ConsistsOf",
    "PropagatesStopTo",
    "StopPropagatedFrom",
    "Upholds",
    "UpheldBy",
    "Conflicts",
    "ConflictedBy",
    "Before",
    "After",
    "Triggers",
    "TriggeredBy",
    "OnSuccess",
    "OnFailure",
    "OnSuccessJobMode",
    "OnFailureJobMode",
    "SuccessAction",
    "FailureAction",
];

const PREFIX: &str = "LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password --all --property=";
const MAX_UNITS_PER_QUERY: usize = 16;
const MAX_UNIT_BYTES: usize = 255;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const QUERY_TIMEOUT: Duration = Duration::from_secs(8);

/// A closed command value for the one reviewed dynamic read-only query.
/// Callers can supply unit identities, never shell syntax or command fragments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemdStopImpactQuery {
    units: Vec<String>,
    command: String,
}

impl SystemdStopImpactQuery {
    pub fn new(units: impl IntoIterator<Item = String>) -> Result<Self, AppError> {
        let units = units.into_iter().collect::<Vec<_>>();
        if units.is_empty() || units.len() > MAX_UNITS_PER_QUERY {
            return Err(policy_error());
        }
        let mut unique = HashSet::with_capacity(units.len());
        for unit in &units {
            validate_unit(unit)?;
            if !unique.insert(unit.as_str()) {
                return Err(policy_error());
            }
        }
        let mut command = String::from(PREFIX);
        command.push_str(&SYSTEMD_STOP_IMPACT_PROPERTIES.join(","));
        command.push_str(" show --");
        for unit in &units {
            command.push_str(" '");
            command.push_str(unit);
            command.push('\'');
        }
        Ok(Self { units, command })
    }

    pub fn single(unit: impl Into<String>) -> Result<Self, AppError> {
        Self::new([unit.into()])
    }

    pub fn units(&self) -> &[String] {
        &self.units
    }

    pub fn command(&self) -> &str {
        &self.command
    }
}

pub struct SystemdStopImpactEngine;

impl SystemdStopImpactEngine {
    pub async fn execute(
        &self,
        session: &dyn RemoteSession,
        query: &SystemdStopImpactQuery,
        cancellation: CancellationToken,
    ) -> Result<String, AppError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        if session.is_closed() {
            return Err(AppError::new(
                ErrorCode::Connection,
                "The connection is closed.",
            ));
        }
        let child = cancellation.child_token();
        let _cancel_on_drop = child.clone().drop_guard();
        let output = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(cancelled()),
            result = tokio::time::timeout(
                QUERY_TIMEOUT,
                session.execute_stop_impact(query, child),
            ) => result.map_err(|_| AppError::new(
                ErrorCode::Timeout,
                "The systemd stop-impact query timed out.",
            ))??,
        };
        if output.len() > MAX_OUTPUT_BYTES || output.contains('\0') {
            return Err(AppError::new(
                ErrorCode::Discovery,
                "The remote command returned invalid or oversized systemd data.",
            ));
        }
        Ok(output)
    }
}

fn validate_unit(unit: &str) -> Result<(), AppError> {
    if unit.is_empty() || unit.len() > MAX_UNIT_BYTES || !unit.is_ascii() {
        return Err(policy_error());
    }
    let bytes = unit.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'.' | b'@' | b'-') {
            index += 1;
            continue;
        }
        if byte == b'\\'
            && index + 3 < bytes.len()
            && bytes[index + 1] == b'x'
            && bytes[index + 2].is_ascii_hexdigit()
            && bytes[index + 3].is_ascii_hexdigit()
        {
            index += 4;
            continue;
        }
        return Err(policy_error());
    }
    let Some(dot) = unit.rfind('.') else {
        return Err(policy_error());
    };
    if dot == 0 || dot + 1 == unit.len() {
        return Err(policy_error());
    }
    Ok(())
}

fn policy_error() -> AppError {
    AppError::new(
        ErrorCode::Policy,
        "The systemd stop-impact query target is invalid.",
    )
}

fn cancelled() -> AppError {
    AppError::new(ErrorCode::Cancelled, "The operation was cancelled.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_only_the_fixed_37_property_command() {
        let query = SystemdStopImpactQuery::new(vec![
            "alpha.service".to_owned(),
            "escaped\\x2dname.service".to_owned(),
        ])
        .unwrap();
        assert_eq!(SYSTEMD_STOP_IMPACT_PROPERTIES.len(), 37);
        assert_eq!(
            query.command(),
            format!(
                "{PREFIX}{} show -- 'alpha.service' 'escaped\\x2dname.service'",
                SYSTEMD_STOP_IMPACT_PROPERTIES.join(",")
            )
        );
        assert!(!query.command().contains("sudo"));
    }

    #[test]
    fn rejects_shell_syntax_duplicates_and_oversized_batches() {
        for unit in [
            "",
            "no-suffix",
            "a.service;id",
            "a.service'",
            "a.service $HOME",
        ] {
            assert_eq!(
                SystemdStopImpactQuery::single(unit).unwrap_err().code,
                ErrorCode::Policy
            );
        }
        assert!(SystemdStopImpactQuery::new(vec!["a.service".into(); 2]).is_err());
        assert!(
            SystemdStopImpactQuery::new((0..17).map(|index| format!("a{index}.service"))).is_err()
        );
    }
}
