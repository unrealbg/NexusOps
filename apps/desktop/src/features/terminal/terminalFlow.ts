import type { TerminalOutputBatch, TerminalSession } from '@nexusops/protocol';

export const INPUT_CHUNK_BYTES = 16 * 1024;
export const MAX_QUEUED_INPUT_BYTES = 256 * 1024;
export const MAX_OUTPUT_BATCH_BYTES = 64 * 1024;

export type TerminalWriteTarget = {
  write(data: Uint8Array, callback: () => void): void;
};

export type SessionUpdateSource = 'poll' | 'rename';

export function isEnded(state: TerminalSession['state']): boolean {
  return state === 'closed' || state === 'failed' || state === 'disconnected';
}

export function mergeTerminalSession(
  current: TerminalSession,
  next: TerminalSession,
  source: SessionUpdateSource,
): TerminalSession {
  if (
    current.id !== next.id ||
    current.hostId !== next.hostId ||
    current.hostSessionId !== next.hostSessionId
  ) {
    return current;
  }
  if (source === 'rename') {
    return next.label === current.label ? current : { ...current, label: next.label };
  }

  // Poll owns lifecycle/error. Rename owns label, and the mounted emulator owns
  // size, so an older poll cannot overwrite either newer local result.
  if (isEnded(current.state)) return current;
  const state = current.state === 'closing' && next.state === 'open' ? current.state : next.state;
  const error = state === current.state && next.error === current.error ? current.error : next.error;
  if (state === current.state && error === current.error) return current;
  return { ...current, state, error };
}

export class BoundedTerminalInput {
  private readonly chunks: Uint8Array[] = [];
  private bytes = 0;
  private stopped = false;
  private flushing: Promise<void> | null = null;

  get bufferedBytes(): number {
    return this.bytes;
  }

  enqueue(bytes: Uint8Array): boolean {
    if (this.stopped || bytes.length === 0) return false;
    if (this.bytes + bytes.length > MAX_QUEUED_INPUT_BYTES) return false;
    for (let offset = 0; offset < bytes.length; offset += INPUT_CHUNK_BYTES) {
      const chunk = bytes.slice(offset, offset + INPUT_CHUNK_BYTES);
      this.chunks.push(chunk);
      this.bytes += chunk.length;
    }
    return true;
  }

  flush(write: (chunk: Uint8Array) => Promise<void>, signal: AbortSignal): Promise<void> {
    if (this.flushing) return this.flushing;
    this.flushing = this.drain(write, signal).finally(() => {
      this.flushing = null;
    });
    return this.flushing;
  }

  stop(): void {
    this.stopped = true;
    this.chunks.length = 0;
    this.bytes = 0;
  }

  private async drain(write: (chunk: Uint8Array) => Promise<void>, signal: AbortSignal): Promise<void> {
    while (!this.stopped && this.chunks.length > 0) {
      const next = this.chunks[0];
      if (!next) return;
      await abortable(write(next), signal);
      if (this.stopped || signal.aborted) return;
      this.chunks.shift();
      this.bytes -= next.length;
    }
  }
}

export async function writeConsumed(
  terminal: TerminalWriteTarget,
  chunksBase64: string[],
  signal: AbortSignal,
): Promise<void> {
  let total = 0;
  for (const encoded of chunksBase64) {
    const bytes = base64ToBytes(encoded);
    total += bytes.length;
    if (bytes.length > INPUT_CHUNK_BYTES || total > MAX_OUTPUT_BATCH_BYTES) {
      throw new Error('The terminal output batch exceeded its renderer limit.');
    }
    await abortable(
      new Promise<void>((resolve) => terminal.write(bytes, resolve)),
      signal,
    );
  }
}

export async function runTerminalPollLoop({
  getSession,
  poll,
  terminal,
  onSession,
  onError,
  signal,
  wait = waitForNextPoll,
}: {
  getSession: () => TerminalSession;
  poll: (session: TerminalSession) => Promise<TerminalOutputBatch>;
  terminal: TerminalWriteTarget;
  onSession: (session: TerminalSession) => void;
  onError: (error: unknown) => void;
  signal: AbortSignal;
  wait?: (signal: AbortSignal) => Promise<void>;
}): Promise<void> {
  let ended = isEnded(getSession().state);
  while (!signal.aborted) {
    try {
      const batch = await abortable(poll(getSession()), signal);
      await writeConsumed(terminal, batch.chunksBase64, signal);
      if (signal.aborted) return;
      onSession(batch.session);
      ended = isEnded(batch.session.state);
      if (ended && batch.outputDrained) return;
    } catch (error) {
      if (signal.aborted || isAbortError(error)) return;
      onError(error);
      if (ended) return;
    }
    try {
      await wait(signal);
    } catch (error) {
      if (signal.aborted || isAbortError(error)) return;
      onError(error);
      if (ended) return;
    }
  }
}

export function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  for (let offset = 0; offset < bytes.length; offset += 8_192) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 8_192));
  }
  return btoa(binary);
}

function base64ToBytes(value: string): Uint8Array {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

function abortable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  if (signal.aborted) return Promise.reject(new DOMException('Cancelled', 'AbortError'));
  return new Promise<T>((resolve, reject) => {
    const abort = () => reject(new DOMException('Cancelled', 'AbortError'));
    signal.addEventListener('abort', abort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener('abort', abort);
        resolve(value);
      },
      (error: unknown) => {
        signal.removeEventListener('abort', abort);
        reject(error);
      },
    );
  });
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'AbortError';
}

function waitForNextPoll(signal: AbortSignal): Promise<void> {
  return abortable(
    new Promise((resolve) => window.setTimeout(resolve, 20)),
    signal,
  );
}
