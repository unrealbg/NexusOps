import { invoke, isTauri } from '@tauri-apps/api/core';
import type {
  AppError,
  CredentialInput,
  Host,
  HostInput,
  HostKeyChallenge,
  HostKeyRotationPlan,
  HostMonitorSample,
  NetworkSnapshot,
  SystemJournalSnapshot,
  ContainerSnapshot,
  ServiceSnapshot,
  ServiceObservationId,
  RemoteOperationPlanId,
  ServiceResetFailedPlan,
  ServiceResetFailedResult,
  ServiceTryRestartPlan,
  ServiceTryRestartResult,
  ServiceReloadPlan,
  ServiceReloadResult,
  ServiceStartPlan,
  ServiceStartResult,
  SystemdStopImpactAssessment,
  SystemdStopImpactInspectionId,
  SshEndpointTrust,
  HostSession,
  TerminalOutputBatch,
  TerminalSession,
  TerminalSize,
  ConflictPolicy,
  DirectoryListing,
  FileOperationPlan,
  LocalSelectionGrant,
  RemoteEntry,
  RemoteTextDocument,
  SftpSessionInfo,
  TransferJob,
  UpdateAnnouncementId,
  UpdateOperationSnapshot,
  VerifiedArtifactId,
} from '@nexusops/protocol';

export function applicationError(error: unknown): AppError {
  if (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    'message' in error &&
    typeof error.message === 'string'
  ) {
    return error as AppError;
  }
  // Library errors may contain connection details. Never show or log arbitrary rejects.
  return {
    code: 'connection',
    message: 'The request could not be completed. Please try again.',
    hostKey: null,
  };
}

async function request<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    throw {
      code: 'connection',
      message:
        'Open NexusOps as a desktop application to manage hosts. This browser preview has no access to SSH or stored credentials.',
      hostKey: null,
    } satisfies AppError;
  }
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw applicationError(error);
  }
}

/** The only frontend/native boundary. There is deliberately no command execution API. */
export const hostApi = {
  list: () => request<Host[]>('list_hosts'),
  save: (input: HostInput, credential: CredentialInput | null) =>
    request<Host>('save_host', { input, credential }),
  delete: (hostId: string) => request<void>('delete_host', { hostId }),
  session: (hostId: string) => request<HostSession>('get_session', { hostId }),
  connect: (hostId: string) => request<void>('connect_host', { hostId }),
  disconnect: (hostId: string) => request<void>('disconnect_host', { hostId }),
  reconnect: (hostId: string) => request<void>('reconnect_host', { hostId }),
  trust: (hostId: string, challenge: HostKeyChallenge) =>
    request<void>('trust_host_key', { hostId, challenge }),
  refresh: (hostId: string) => request<void>('refresh_host', { hostId }),
};

/** Narrow native updater boundary. Download and install accept only their current opaque authorities. */
export const updateApi = {
  state: () => request<UpdateOperationSnapshot>('get_update_state'),
  check: () => request<UpdateOperationSnapshot>('check_for_update'),
  download: (announcementId: UpdateAnnouncementId) =>
    request<UpdateOperationSnapshot>('download_announced_update', { announcementId }),
  install: (verifiedArtifactId: VerifiedArtifactId) =>
    request<void>('install_verified_update', { verifiedArtifactId }),
};

/** Session-bound, fixed-command monitoring boundary. */
export const monitorApi = {
  sample: (hostId: string, hostSessionId: string) =>
    request<HostMonitorSample>('sample_host_monitor', { hostId, hostSessionId }),
};

/** Fixed read-only, session-bound systemd inventory; filtering stays in the component. */
export const servicesApi = {
  list: (hostId: string, hostSessionId: string) =>
    request<ServiceSnapshot>('list_host_services', { hostId, hostSessionId }),
  assessStopImpact: (
    hostId: string,
    hostSessionId: string,
    inspectionId: SystemdStopImpactInspectionId,
  ) => request<SystemdStopImpactAssessment>('assess_service_stop_impact', {
    hostId, hostSessionId, inspectionId,
  }),
  planResetFailed: (
    hostId: string,
    hostSessionId: string,
    serviceObservationId: ServiceObservationId,
  ) => request<ServiceResetFailedPlan>('plan_service_reset_failed', {
    hostId, hostSessionId, serviceObservationId,
  }),
  discardResetFailed: (
    hostId: string,
    hostSessionId: string,
    planId: RemoteOperationPlanId,
  ) => request<boolean>('discard_service_reset_failed', { hostId, hostSessionId, planId }),
  executeResetFailed: (
    hostId: string,
    hostSessionId: string,
    planId: RemoteOperationPlanId,
  ) => request<ServiceResetFailedResult>('execute_service_reset_failed', {
    hostId, hostSessionId, planId,
  }),
  planTryRestart: (
    hostId: string,
    hostSessionId: string,
    serviceObservationId: ServiceObservationId,
  ) => request<ServiceTryRestartPlan>('plan_service_try_restart', {
    hostId, hostSessionId, serviceObservationId,
  }),
  discardTryRestart: (
    hostId: string,
    hostSessionId: string,
    planId: RemoteOperationPlanId,
  ) => request<boolean>('discard_service_try_restart', { hostId, hostSessionId, planId }),
  executeTryRestart: (
    hostId: string,
    hostSessionId: string,
    planId: RemoteOperationPlanId,
  ) => request<ServiceTryRestartResult>('execute_service_try_restart', {
    hostId, hostSessionId, planId,
  }),
  planReload: (
    hostId: string,
    hostSessionId: string,
    serviceObservationId: ServiceObservationId,
  ) => request<ServiceReloadPlan>('plan_service_reload', {
    hostId, hostSessionId, serviceObservationId,
  }),
  discardReload: (
    hostId: string,
    hostSessionId: string,
    planId: RemoteOperationPlanId,
  ) => request<boolean>('discard_service_reload', { hostId, hostSessionId, planId }),
  executeReload: (
    hostId: string,
    hostSessionId: string,
    planId: RemoteOperationPlanId,
  ) => request<ServiceReloadResult>('execute_service_reload', {
    hostId, hostSessionId, planId,
  }),
  planStart: (
    hostId: string,
    hostSessionId: string,
    serviceObservationId: ServiceObservationId,
  ) => request<ServiceStartPlan>('plan_service_start', {
    hostId, hostSessionId, serviceObservationId,
  }),
  discardStart: (
    hostId: string,
    hostSessionId: string,
    planId: RemoteOperationPlanId,
  ) => request<boolean>('discard_service_start', { hostId, hostSessionId, planId }),
  executeStart: (
    hostId: string,
    hostSessionId: string,
    planId: RemoteOperationPlanId,
  ) => request<ServiceStartResult>('execute_service_start', {
    hostId, hostSessionId, planId,
  }),
};

/** One fixed read-only network observation; no renderer-supplied remote arguments. */
export const networkApi = {
  list: (hostId: string, hostSessionId: string) =>
    request<NetworkSnapshot>('list_host_network', { hostId, hostSessionId }),
};

/** Sensitive, session-owned journal snapshot; never cache or persist the result. */
export const logsApi = {
  list: (hostId: string, hostSessionId: string) =>
    request<SystemJournalSnapshot>('list_host_logs', { hostId, hostSessionId }),
};

/** One fixed, session-bound Docker system-socket read; metadata remains component-local. */
export const containersApi = {
  list: (hostId: string, hostSessionId: string) =>
    request<ContainerSnapshot>('list_host_containers', { hostId, hostSessionId }),
};

type TerminalOwnership = Pick<TerminalSession, 'hostId' | 'hostSessionId' | 'id'>;

/** Local endpoint metadata only; no connection, credential access, or remote observation. */
export const securityApi = {
  get: (hostId: string) => request<SshEndpointTrust>('get_host_ssh_trust', { hostId }),
  planRotation: (hostId: string) => request<HostKeyRotationPlan>('plan_host_key_rotation', { hostId }),
  executeRotation: (hostId: string, planId: string) =>
    request<void>('execute_host_key_rotation', { hostId, planId }),
};

function ownershipArgs(session: TerminalOwnership) {
  return {
    hostId: session.hostId,
    hostSessionId: session.hostSessionId,
    terminalId: session.id,
  };
}

/** Dedicated PTY boundary. It cannot execute a command outside an interactive terminal. */
export const terminalApi = {
  list: (hostId: string) => request<TerminalSession[]>('list_terminals', { hostId }),
  open: (hostId: string, size: TerminalSize) =>
    request<TerminalSession>('open_terminal', { hostId, size }),
  poll: (session: TerminalOwnership) =>
    request<TerminalOutputBatch>('poll_terminal', ownershipArgs(session)),
  write: (session: TerminalOwnership, dataBase64: string) =>
    request<void>('write_terminal', { ...ownershipArgs(session), dataBase64 }),
  resize: (session: TerminalOwnership, size: TerminalSize) =>
    request<void>('resize_terminal', { ...ownershipArgs(session), size }),
  rename: (session: TerminalOwnership, label: string) =>
    request<TerminalSession>('rename_terminal', { ...ownershipArgs(session), label }),
  close: (session: TerminalOwnership) =>
    request<void>('close_terminal', ownershipArgs(session)),
};

type SftpOwnership = Pick<SftpSessionInfo, 'hostId' | 'hostSessionId' | 'id'>;
function sftpArgs(session: SftpOwnership) {
  return { hostId: session.hostId, hostSessionId: session.hostSessionId, sftpSessionId: session.id };
}

/** Narrow Files boundary: transfer payload/local filesystem bytes remain native-side;
 * bounded remote editor text is the explicit component-local exception. */
export const filesApi = {
  open: (hostId: string) => request<SftpSessionInfo>('open_sftp', { hostId }),
  list: (session: SftpOwnership, path: string) => request<DirectoryListing>('list_remote_directory', { ...sftpArgs(session), path }),
  properties: (session: SftpOwnership, path: string) => request<RemoteEntry>('remote_properties', { ...sftpArgs(session), path }),
  openText: (session: SftpOwnership, path: string) => request<RemoteTextDocument>('open_remote_text_file', { ...sftpArgs(session), path }),
  planTextSave: (session: SftpOwnership, documentId: string, text: string) => request<FileOperationPlan>('plan_remote_text_save', { ...sftpArgs(session), documentId, text }),
  discardTextDocument: (session: SftpOwnership, documentId: string) => request<void>('discard_remote_text_document', { ...sftpArgs(session), documentId }),
  chooseUploadFiles: (session: SftpOwnership) => request<LocalSelectionGrant | null>('choose_upload_files', sftpArgs(session)),
  chooseDownloadDirectory: (session: SftpOwnership) => request<LocalSelectionGrant | null>('choose_download_directory', sftpArgs(session)),
  planUpload: (session: SftpOwnership, grantId: string, remoteDirectory: string, conflictPolicy: ConflictPolicy) => request<FileOperationPlan>('plan_upload', { request: { ...sftpArgs(session), grantId, remoteDirectory, conflictPolicy } }),
  planDownload: (session: SftpOwnership, grantId: string, remotePaths: string[], conflictPolicy: ConflictPolicy) => request<FileOperationPlan>('plan_download', { request: { ...sftpArgs(session), grantId, remotePaths, conflictPolicy } }),
  planCreateDirectory: (session: SftpOwnership, parent: string, name: string) => request<FileOperationPlan>('plan_create_directory', { ...sftpArgs(session), parent, name }),
  planRename: (session: SftpOwnership, source: string, newName: string) => request<FileOperationPlan>('plan_rename', { ...sftpArgs(session), source, newName }),
  planDelete: (session: SftpOwnership, path: string) => request<FileOperationPlan>('plan_delete', { ...sftpArgs(session), path }),
  execute: (session: SftpOwnership, planId: string) => request<TransferJob[]>('execute_file_plan', { ...sftpArgs(session), planId }),
  discardPlan: (session: SftpOwnership, planId: string) => request<void>('discard_file_plan', { ...sftpArgs(session), planId }),
  discardEditorPlan: (session: SftpOwnership, planId: string) => request<boolean>('discard_file_plan', { ...sftpArgs(session), planId }),
  discardGrant: (session: SftpOwnership, grantId: string) => request<void>('discard_local_grant', { ...sftpArgs(session), grantId }),
  transfers: (hostId?: string) => request<TransferJob[]>('list_transfers', { hostId: hostId ?? null }),
  cancel: (session: SftpOwnership, jobId: string) => request<void>('cancel_transfer', { ...sftpArgs(session), jobId }),
  planRetry: (session: SftpOwnership, jobId: string) => request<FileOperationPlan>('plan_retry_transfer', { ...sftpArgs(session), jobId }),
};
