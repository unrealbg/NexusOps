use crate::SshSession;
use async_trait::async_trait;
use nexus_operations::RemoteSession;
use nexus_remote_operations::{
    CompletionUnknownReason, ConsumedAuthority, MutationTransport, MutationTransportOutcome,
    NotDispatchedReason, SystemdResetFailed,
};
use russh::ChannelMsg;
use std::time::Duration;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

const CHANNEL_OPEN_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_COMPLETION_TIMEOUT: Duration = Duration::from_secs(15);
const CHANNEL_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const OUTPUT_LIMIT: usize = 8 * 1024;
const COMMAND_PREFIX: &str = "LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password reset-failed -- ";

fn command(authority: &ConsumedAuthority<SystemdResetFailed>) -> String {
    format!("{COMMAND_PREFIX}{}", authority.target().as_str())
}

#[async_trait]
impl MutationTransport<SystemdResetFailed> for SshSession {
    async fn dispatch(
        &self,
        authority: &ConsumedAuthority<SystemdResetFailed>,
        cancellation: CancellationToken,
    ) -> MutationTransportOutcome {
        if self.is_closed() {
            return MutationTransportOutcome::NotDispatched(NotDispatchedReason::Connection);
        }
        if cancellation.is_cancelled() {
            return MutationTransportOutcome::NotDispatched(NotDispatchedReason::Cancelled);
        }
        let open = async {
            let handle = self.handle.lock().await;
            handle.channel_open_session().await
        };
        let mut channel = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                return MutationTransportOutcome::NotDispatched(NotDispatchedReason::Cancelled);
            }
            _ = self.lifetime.cancelled() => {
                return MutationTransportOutcome::NotDispatched(NotDispatchedReason::Connection);
            }
            result = timeout(CHANNEL_OPEN_TIMEOUT, open) => match result {
                Err(_) => return MutationTransportOutcome::NotDispatched(NotDispatchedReason::Timeout),
                Ok(Err(_)) => return MutationTransportOutcome::NotDispatched(NotDispatchedReason::Connection),
                Ok(Ok(channel)) => channel,
            }
        };

        // From immediately before this request onward, a dropped response can
        // no longer prove that the remote mutation did not run.
        let request = async {
            if channel.exec(true, command(authority)).await.is_err() {
                return MutationTransportOutcome::CompletionUnknown(
                    CompletionUnknownReason::ConnectionLost,
                );
            }
            let mut accepted = false;
            let mut execution_evidence = false;
            let mut output_bytes = 0usize;
            loop {
                match channel.wait().await {
                    Some(ChannelMsg::Success) if !accepted => accepted = true,
                    Some(ChannelMsg::Success) => {
                        return MutationTransportOutcome::CompletionUnknown(
                            CompletionUnknownReason::ConnectionLost,
                        );
                    }
                    Some(ChannelMsg::Failure) if !accepted && !execution_evidence => {
                        return MutationTransportOutcome::NotDispatched(
                            NotDispatchedReason::Rejected,
                        );
                    }
                    Some(ChannelMsg::Failure) => {
                        return MutationTransportOutcome::CompletionUnknown(
                            CompletionUnknownReason::ConnectionLost,
                        );
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        return MutationTransportOutcome::CompletionConfirmed {
                            success: exit_status == 0,
                        };
                    }
                    Some(ChannelMsg::ExitSignal { .. }) => {
                        return MutationTransportOutcome::CompletionConfirmed { success: false };
                    }
                    Some(ChannelMsg::Data { data })
                    | Some(ChannelMsg::ExtendedData { data, .. }) => {
                        execution_evidence = true;
                        output_bytes = output_bytes.saturating_add(data.len());
                        if output_bytes > OUTPUT_LIMIT {
                            return MutationTransportOutcome::CompletionUnknown(
                                CompletionUnknownReason::OutputLimit,
                            );
                        }
                    }
                    Some(ChannelMsg::Eof) => {}
                    Some(ChannelMsg::Close) | None => {
                        return MutationTransportOutcome::CompletionUnknown(
                            CompletionUnknownReason::ConnectionLost,
                        );
                    }
                    Some(_) if !accepted => {
                        return MutationTransportOutcome::CompletionUnknown(
                            CompletionUnknownReason::ConnectionLost,
                        );
                    }
                    Some(_) => {}
                }
            }
        };
        let outcome = tokio::select! {
            biased;
            _ = cancellation.cancelled() => MutationTransportOutcome::CompletionUnknown(
                CompletionUnknownReason::Cancelled,
            ),
            _ = self.lifetime.cancelled() => MutationTransportOutcome::CompletionUnknown(
                CompletionUnknownReason::ConnectionLost,
            ),
            result = timeout(REQUEST_COMPLETION_TIMEOUT, request) => match result {
                Ok(outcome) => outcome,
                Err(_) => MutationTransportOutcome::CompletionUnknown(
                    CompletionUnknownReason::Timeout,
                ),
            }
        };
        let _ = timeout(CHANNEL_CLOSE_TIMEOUT, channel.close()).await;
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_model::{HostId, HostSessionId};
    use nexus_remote_operations::{
        AuthorityBinding, PlanDraft, RemoteOperationFoundation, SystemdResetFailedPreconditions,
        SystemdServiceUnitName,
    };

    #[test]
    fn command_is_exact_and_contains_only_the_validated_unit() {
        let foundation = RemoteOperationFoundation::default();
        let binding = AuthorityBinding {
            host_id: HostId::new(),
            host_session_id: HostSessionId::new(),
            generation: 3,
        };
        let receipt = foundation
            .plan::<SystemdResetFailed>(
                PlanDraft::new(
                    binding,
                    SystemdServiceUnitName::parse("worker@blue-1.service").unwrap(),
                    SystemdResetFailedPreconditions,
                    (),
                ),
                binding,
            )
            .unwrap();
        // A unit test in the authority crate validates all rejected shell-like
        // inputs; the exact fixed command is additionally guarded by policy.
        assert_eq!(receipt.binding, binding);
        assert_eq!(
            COMMAND_PREFIX,
            "LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password reset-failed -- "
        );
    }
}
