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
