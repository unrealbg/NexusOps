use crate::{ReadOnlyCommand, RemoteSession};
use nexus_model::{AppError, ErrorCode, Operation, OperationRisk};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// A reviewable plan. The command is private so metadata cannot substitute a shell.
pub struct OperationPlan {
    pub operation: Operation,
    command: ReadOnlyCommand,
}

impl OperationPlan {
    pub const fn command(&self) -> ReadOnlyCommand {
        self.command
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RollbackStatus {
    /// Read-only operations have no remote changes to undo.
    NotRequired,
}

/// Goal 01's policy admits only reviewed read-only operations.
/// Verification is transport-level here; providers verify domain-specific output.
pub struct OperationEngine {
    timeout: Duration,
    max_output_bytes: usize,
}

impl Default for OperationEngine {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(8),
            max_output_bytes: 64 * 1024,
        }
    }
}

impl OperationEngine {
    pub fn plan(&self, command: ReadOnlyCommand) -> OperationPlan {
        OperationPlan {
            operation: command.operation(),
            command,
        }
    }

    pub fn validate(&self, plan: &OperationPlan) -> Result<(), AppError> {
        if plan.operation.risk != OperationRisk::ReadOnly
            || plan.operation.kind != plan.command.kind()
        {
            return Err(AppError::new(
                ErrorCode::Policy,
                "Only approved read-only discovery operations are available.",
            ));
        }
        Ok(())
    }

    pub async fn execute(
        &self,
        session: &dyn RemoteSession,
        plan: &OperationPlan,
        cancellation: CancellationToken,
    ) -> Result<String, AppError> {
        self.validate(plan)?;
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
            result = tokio::time::timeout(self.timeout, session.execute(plan.command, child)) => {
                result.map_err(|_| AppError::new(ErrorCode::Timeout, "The discovery operation timed out."))??
            }
        };
        self.verify(plan, &output)?;
        Ok(output)
    }

    pub fn verify(&self, plan: &OperationPlan, output: &str) -> Result<(), AppError> {
        self.validate(plan)?;
        if output.len() > self.max_output_bytes || output.contains('\0') {
            return Err(AppError::new(
                ErrorCode::Discovery,
                "The remote command returned invalid or oversized discovery data.",
            ));
        }
        Ok(())
    }

    /// Future write operations must introduce an explicit compensating plan.
    /// The current allowlist cannot mutate a host, so rollback performs no work.
    pub fn rollback(&self, plan: &OperationPlan) -> Result<RollbackStatus, AppError> {
        self.validate(plan)?;
        Ok(RollbackStatus::NotRequired)
    }
}

fn cancelled() -> AppError {
    AppError::new(ErrorCode::Cancelled, "The operation was cancelled.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Session {
        calls: AtomicUsize,
        output: String,
        stall: bool,
    }

    #[async_trait]
    impl RemoteSession for Session {
        async fn execute(
            &self,
            _: ReadOnlyCommand,
            _: CancellationToken,
        ) -> Result<String, AppError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.stall {
                std::future::pending::<()>().await;
            }
            Ok(self.output.clone())
        }

        async fn disconnect(&self) -> Result<(), AppError> {
            Ok(())
        }
        fn is_closed(&self) -> bool {
            false
        }
    }

    fn session(output: &str) -> Session {
        Session {
            calls: AtomicUsize::new(0),
            output: output.into(),
            stall: false,
        }
    }

    #[tokio::test]
    async fn rejects_every_write_risk_before_execution() {
        let engine = OperationEngine::default();
        let session = session("host\n");
        for risk in [
            OperationRisk::Low,
            OperationRisk::Moderate,
            OperationRisk::High,
            OperationRisk::Destructive,
        ] {
            let mut plan = engine.plan(ReadOnlyCommand::Hostname);
            plan.operation.risk = risk;
            let error = engine
                .execute(&session, &plan, CancellationToken::new())
                .await
                .expect_err("writes must be rejected");
            assert_eq!(error.code, ErrorCode::Policy);
        }
        assert_eq!(session.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn risks_are_ordered_and_read_only_needs_no_rollback() {
        assert!(OperationRisk::ReadOnly < OperationRisk::Low);
        assert!(OperationRisk::Low < OperationRisk::Moderate);
        assert!(OperationRisk::Moderate < OperationRisk::High);
        assert!(OperationRisk::High < OperationRisk::Destructive);
        let engine = OperationEngine::default();
        assert_eq!(
            engine
                .rollback(&engine.plan(ReadOnlyCommand::Hostname))
                .expect("read only"),
            RollbackStatus::NotRequired
        );
    }

    #[tokio::test]
    async fn cancellation_prevents_execution() {
        let session = session("host");
        let engine = OperationEngine::default();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let error = engine
            .execute(
                &session,
                &engine.plan(ReadOnlyCommand::Hostname),
                cancellation,
            )
            .await
            .expect_err("cancelled");
        assert_eq!(error.code, ErrorCode::Cancelled);
        assert_eq!(session.calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn stalled_command_has_a_deadline() {
        let mut session = session("");
        session.stall = true;
        let engine = OperationEngine {
            timeout: Duration::from_millis(5),
            max_output_bytes: 10,
        };
        let error = engine
            .execute(
                &session,
                &engine.plan(ReadOnlyCommand::Hostname),
                CancellationToken::new(),
            )
            .await
            .expect_err("deadline");
        assert_eq!(error.code, ErrorCode::Timeout);
    }

    #[tokio::test]
    async fn verifies_output_bound_and_plan_identity() {
        let engine = OperationEngine {
            timeout: Duration::from_secs(1),
            max_output_bytes: 4,
        };
        let plan = engine.plan(ReadOnlyCommand::Hostname);
        let error = engine
            .execute(&session("oversized"), &plan, CancellationToken::new())
            .await
            .expect_err("bounded output");
        assert_eq!(error.code, ErrorCode::Discovery);
        assert!(engine.verify(&plan, "a\0b").is_err());
        let mut changed = engine.plan(ReadOnlyCommand::Hostname);
        changed.operation.kind = "unreviewed.operation".into();
        assert!(engine.validate(&changed).is_err());
    }
}
