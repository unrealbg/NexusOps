import { invoke, isTauri } from '@tauri-apps/api/core';
import type {
  AppError,
  CredentialInput,
  Host,
  HostInput,
  HostKeyChallenge,
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

type TerminalOwnership = Pick<TerminalSession, 'hostId' | 'hostSessionId' | 'id'>;

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

/** Narrow Files boundary: payload bytes and local paths never enter the renderer. */
export const filesApi = {
  open: (hostId: string) => request<SftpSessionInfo>('open_sftp', { hostId }),
  list: (session: SftpOwnership, path: string) => request<DirectoryListing>('list_remote_directory', { ...sftpArgs(session), path }),
  properties: (session: SftpOwnership, path: string) => request<RemoteEntry>('remote_properties', { ...sftpArgs(session), path }),
  openText: (session: SftpOwnership, path: string) => request<RemoteTextDocument>('open_remote_text_file', { ...sftpArgs(session), path }),
  planTextSave: (session: SftpOwnership, documentId: string, text: string) => request<FileOperationPlan>('plan_remote_text_save', { ...sftpArgs(session), documentId, text }),
  chooseUploadFiles: (session: SftpOwnership) => request<LocalSelectionGrant | null>('choose_upload_files', sftpArgs(session)),
  chooseDownloadDirectory: (session: SftpOwnership) => request<LocalSelectionGrant | null>('choose_download_directory', sftpArgs(session)),
  planUpload: (session: SftpOwnership, grantId: string, remoteDirectory: string, conflictPolicy: ConflictPolicy) => request<FileOperationPlan>('plan_upload', { request: { ...sftpArgs(session), grantId, remoteDirectory, conflictPolicy } }),
  planDownload: (session: SftpOwnership, grantId: string, remotePaths: string[], conflictPolicy: ConflictPolicy) => request<FileOperationPlan>('plan_download', { request: { ...sftpArgs(session), grantId, remotePaths, conflictPolicy } }),
  planCreateDirectory: (session: SftpOwnership, parent: string, name: string) => request<FileOperationPlan>('plan_create_directory', { ...sftpArgs(session), parent, name }),
  planRename: (session: SftpOwnership, source: string, newName: string) => request<FileOperationPlan>('plan_rename', { ...sftpArgs(session), source, newName }),
  planDelete: (session: SftpOwnership, path: string) => request<FileOperationPlan>('plan_delete', { ...sftpArgs(session), path }),
  execute: (session: SftpOwnership, planId: string) => request<TransferJob[]>('execute_file_plan', { ...sftpArgs(session), planId }),
  discardPlan: (session: SftpOwnership, planId: string) => request<void>('discard_file_plan', { ...sftpArgs(session), planId }),
  discardGrant: (session: SftpOwnership, grantId: string) => request<void>('discard_local_grant', { ...sftpArgs(session), grantId }),
  transfers: (hostId?: string) => request<TransferJob[]>('list_transfers', { hostId: hostId ?? null }),
  cancel: (session: SftpOwnership, jobId: string) => request<void>('cancel_transfer', { ...sftpArgs(session), jobId }),
  planRetry: (session: SftpOwnership, jobId: string) => request<FileOperationPlan>('plan_retry_transfer', { ...sftpArgs(session), jobId }),
};
