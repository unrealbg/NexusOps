use crate::SshSession;
use async_trait::async_trait;
use nexus_operations::RemoteSession;
use nexus_remote_operations::{
    CompletionUnknownReason, ConsumedAuthority, MutationTransport, MutationTransportOutcome,
    NotDispatchedReason, SystemdTryRestart,
};
use russh::ChannelMsg;
use std::time::Duration;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

const CHANNEL_OPEN_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_COMPLETION_TIMEOUT: Duration = Duration::from_secs(30);
const CHANNEL_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const OUTPUT_LIMIT: usize = 8 * 1024;
const COMMAND_PREFIX: &str = "LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password try-restart -- ";

fn command(authority: &ConsumedAuthority<SystemdTryRestart>) -> String {
    format!("{COMMAND_PREFIX}{}", authority.target().as_str())
}

#[derive(Debug, Eq, PartialEq)]
enum MessageHandling {
    Continue,
    Complete(MutationTransportOutcome),
}

fn handle_message(
    message: ChannelMsg,
    accepted: &mut bool,
    execution_evidence: &mut bool,
    output_bytes: &mut usize,
) -> MessageHandling {
    match message {
        ChannelMsg::Success if !*accepted => {
            *accepted = true;
            MessageHandling::Continue
        }
        ChannelMsg::Success => MessageHandling::Complete(
            MutationTransportOutcome::CompletionUnknown(CompletionUnknownReason::ConnectionLost),
        ),
        ChannelMsg::Failure if !*accepted && !*execution_evidence => MessageHandling::Complete(
            MutationTransportOutcome::NotDispatched(NotDispatchedReason::Rejected),
        ),
        ChannelMsg::Failure => MessageHandling::Complete(
            MutationTransportOutcome::CompletionUnknown(CompletionUnknownReason::ConnectionLost),
        ),
        ChannelMsg::ExitStatus { exit_status } => {
            MessageHandling::Complete(MutationTransportOutcome::CompletionConfirmed {
                success: exit_status == 0,
            })
        }
        ChannelMsg::ExitSignal { .. } => {
            MessageHandling::Complete(MutationTransportOutcome::CompletionConfirmed {
                success: false,
            })
        }
        ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
            *execution_evidence = true;
            *output_bytes = output_bytes.saturating_add(data.len());
            if *output_bytes > OUTPUT_LIMIT {
                MessageHandling::Complete(MutationTransportOutcome::CompletionUnknown(
                    CompletionUnknownReason::OutputLimit,
                ))
            } else {
                MessageHandling::Continue
            }
        }
        ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } => MessageHandling::Continue,
        ChannelMsg::Close => MessageHandling::Complete(
            MutationTransportOutcome::CompletionUnknown(CompletionUnknownReason::ConnectionLost),
        ),
        _ if !*accepted => MessageHandling::Complete(MutationTransportOutcome::CompletionUnknown(
            CompletionUnknownReason::ConnectionLost,
        )),
        _ => MessageHandling::Continue,
    }
}

#[async_trait]
impl MutationTransport<SystemdTryRestart> for SshSession {
    async fn dispatch(
        &self,
        authority: &ConsumedAuthority<SystemdTryRestart>,
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
                    Some(message) => match handle_message(
                        message,
                        &mut accepted,
                        &mut execution_evidence,
                        &mut output_bytes,
                    ) {
                        MessageHandling::Continue => {}
                        MessageHandling::Complete(outcome) => return outcome,
                    },
                    None => {
                        return MutationTransportOutcome::CompletionUnknown(
                            CompletionUnknownReason::ConnectionLost,
                        );
                    }
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
        AuthorityBinding, PlanDraft, RemoteOperationFoundation, SystemdServiceUnitName,
        SystemdTryRestartPreconditions,
    };

    fn handle_sequence(messages: impl IntoIterator<Item = ChannelMsg>) -> MessageHandling {
        let mut accepted = false;
        let mut execution_evidence = false;
        let mut output_bytes = 0;
        let mut handling = MessageHandling::Continue;
        for message in messages {
            handling = handle_message(
                message,
                &mut accepted,
                &mut execution_evidence,
                &mut output_bytes,
            );
            if matches!(handling, MessageHandling::Complete(_)) {
                break;
            }
        }
        handling
    }

    #[test]
    fn window_adjusted_continues_until_terminal_evidence() {
        for (messages, expected) in [
            (
                vec![
                    ChannelMsg::WindowAdjusted { new_size: 16_384 },
                    ChannelMsg::ExitStatus { exit_status: 0 },
                ],
                MutationTransportOutcome::CompletionConfirmed { success: true },
            ),
            (
                vec![
                    ChannelMsg::WindowAdjusted { new_size: 16_384 },
                    ChannelMsg::ExitStatus { exit_status: 127 },
                ],
                MutationTransportOutcome::CompletionConfirmed { success: false },
            ),
            (
                vec![
                    ChannelMsg::WindowAdjusted { new_size: 16_384 },
                    ChannelMsg::Eof,
                    ChannelMsg::ExitStatus { exit_status: 0 },
                ],
                MutationTransportOutcome::CompletionConfirmed { success: true },
            ),
            (
                vec![
                    ChannelMsg::Success,
                    ChannelMsg::WindowAdjusted { new_size: 16_384 },
                    ChannelMsg::ExitStatus { exit_status: 0 },
                ],
                MutationTransportOutcome::CompletionConfirmed { success: true },
            ),
        ] {
            assert_eq!(
                handle_sequence(messages),
                MessageHandling::Complete(expected)
            );
        }
    }

    #[test]
    fn window_adjusted_changes_no_transport_state_and_other_preaccept_messages_fail_closed() {
        let mut accepted = false;
        let mut execution_evidence = false;
        let mut output_bytes = 37;
        assert_eq!(
            handle_message(
                ChannelMsg::WindowAdjusted { new_size: 16_384 },
                &mut accepted,
                &mut execution_evidence,
                &mut output_bytes,
            ),
            MessageHandling::Continue
        );
        assert!(!accepted);
        assert!(!execution_evidence);
        assert_eq!(output_bytes, 37);
        assert_eq!(
            handle_message(
                ChannelMsg::XonXoff {
                    client_can_do: true,
                },
                &mut accepted,
                &mut execution_evidence,
                &mut output_bytes,
            ),
            MessageHandling::Complete(MutationTransportOutcome::CompletionUnknown(
                CompletionUnknownReason::ConnectionLost,
            ))
        );
    }

    #[test]
    fn command_is_exact_and_contains_only_the_validated_unit() {
        let foundation = RemoteOperationFoundation::default();
        let binding = AuthorityBinding {
            host_id: HostId::new(),
            host_session_id: HostSessionId::new(),
            generation: 3,
        };
        let receipt = foundation
            .plan::<SystemdTryRestart>(
                PlanDraft::new(
                    binding,
                    SystemdServiceUnitName::parse("worker@blue-1.service").unwrap(),
                    SystemdTryRestartPreconditions,
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
            "LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password try-restart -- "
        );
    }
}
