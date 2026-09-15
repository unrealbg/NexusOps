import type { TerminalOutputBatch, TerminalSession } from '@nexusops/protocol';
import { describe, expect, test, vi } from 'vitest';
import {
  BoundedTerminalInput,
  MAX_QUEUED_INPUT_BYTES,
  mergeTerminalSession,
  runTerminalPollLoop,
} from './terminalFlow';

function session(label = 'Terminal 1', state: TerminalSession['state'] = 'open'): TerminalSession {
  return {
    id: 'terminal-a',
    hostId: 'host-a',
    hostSessionId: 'connection-a',
    label,
    state,
    size: { columns: 80, rows: 24, pixelWidth: 0, pixelHeight: 0 },
    error: null,
  };
}

function encoded(value: string): string {
  return btoa(value);
}

async function eventually(assertion: () => void): Promise<void> {
  await vi.waitFor(assertion, { timeout: 1_000, interval: 1 });
}

describe('terminal renderer flow control', () => {
  test('waits for xterm consumption before polling again and preserves byte order', async () => {
    const callbacks: Array<() => void> = [];
    const writes: Uint8Array[] = [];
    const terminal = {
      write(bytes: Uint8Array, callback: () => void) {
        writes.push(bytes);
        callbacks.push(callback);
      },
    };
    const batches: TerminalOutputBatch[] = [
      { session: session(), chunksBase64: [encoded('first-'), encoded('second-')], outputDrained: false },
      {
        session: session('Terminal 1', 'closed'),
        chunksBase64: [encoded('final-marker')],
        outputDrained: true,
      },
    ];
    const poll = vi.fn(async () => batches.shift()!);
    const waits: Array<() => void> = [];
    const controller = new AbortController();
    const loop = runTerminalPollLoop({
      getSession: () => session(),
      poll,
      terminal,
      onSession: vi.fn(),
      onError: vi.fn(),
      signal: controller.signal,
      wait: () => new Promise((resolve) => waits.push(resolve)),
    });

    await eventually(() => expect(writes).toHaveLength(1));
    expect(poll).toHaveBeenCalledTimes(1);
    callbacks.shift()?.();
    await eventually(() => expect(writes).toHaveLength(2));
    expect(poll).toHaveBeenCalledTimes(1);
    callbacks.shift()?.();
    await eventually(() => expect(waits).toHaveLength(1));
    waits.shift()?.();
    await eventually(() => expect(poll).toHaveBeenCalledTimes(2));
    expect(writes).toHaveLength(3);
    callbacks.shift()?.();
    await loop;

    const actual = writes.flatMap((bytes) => [...bytes]);
    expect(new TextDecoder().decode(new Uint8Array(actual))).toBe('first-second-final-marker');
  });

  test('cancellation releases a poll paused on a stalled emulator callback', async () => {
    const terminal = { write: vi.fn((_bytes: Uint8Array, _callback: () => void) => {}) };
    const poll = vi.fn(async () => ({
      session: session('Terminal 1', 'closed'),
      chunksBase64: [encoded('unconsumed')],
      outputDrained: true,
    }));
    const onSession = vi.fn();
    const controller = new AbortController();
    const loop = runTerminalPollLoop({
      getSession: () => session(),
      poll,
      terminal,
      onSession,
      onError: vi.fn(),
      signal: controller.signal,
    });
    await eventually(() => expect(terminal.write).toHaveBeenCalledTimes(1));
    controller.abort();
    await loop;
    expect(poll).toHaveBeenCalledTimes(1);
    expect(onSession).not.toHaveBeenCalled();
  });

  test('a poll resolving after effect cleanup cannot publish stale state', async () => {
    let resolvePoll!: (batch: TerminalOutputBatch) => void;
    const poll = vi.fn(() => new Promise<TerminalOutputBatch>((resolve) => (resolvePoll = resolve)));
    const onSession = vi.fn();
    const controller = new AbortController();
    const loop = runTerminalPollLoop({
      getSession: () => session(),
      poll,
      terminal: { write: vi.fn() },
      onSession,
      onError: vi.fn(),
      signal: controller.signal,
    });
    await eventually(() => expect(poll).toHaveBeenCalledTimes(1));
    controller.abort();
    resolvePoll({ session: session('stale', 'closed'), chunksBase64: [], outputDrained: true });
    await loop;
    expect(onSession).not.toHaveBeenCalled();
    expect(poll).toHaveBeenCalledTimes(1);
  });

  test('bounds aggregate input and resumes a slow writer without loss or reordering', async () => {
    const queue = new BoundedTerminalInput();
    const accepted = Uint8Array.from({ length: MAX_QUEUED_INPUT_BYTES }, (_, index) => index % 251);
    expect(queue.enqueue(accepted)).toBe(true);
    expect(queue.bufferedBytes).toBe(MAX_QUEUED_INPUT_BYTES);
    expect(queue.enqueue(Uint8Array.of(1))).toBe(false);

    let releaseFirst!: () => void;
    const delivered: Uint8Array[] = [];
    let call = 0;
    const flush = queue.flush(async (chunk) => {
      delivered.push(chunk);
      call += 1;
      if (call === 1) await new Promise<void>((resolve) => (releaseFirst = resolve));
    }, new AbortController().signal);
    await eventually(() => expect(delivered).toHaveLength(1));
    expect(queue.bufferedBytes).toBe(MAX_QUEUED_INPUT_BYTES);
    releaseFirst();
    await flush;

    const actual = delivered.flatMap((bytes) => [...bytes]);
    expect(new Uint8Array(actual)).toEqual(accepted);
    expect(queue.bufferedBytes).toBe(0);
  });

  test('input cancellation clears accepted queued bytes without waiting for a stalled writer', async () => {
    const queue = new BoundedTerminalInput();
    expect(queue.enqueue(new Uint8Array(32 * 1024))).toBe(true);
    const controller = new AbortController();
    const flush = queue.flush(() => new Promise<void>(() => {}), controller.signal);
    controller.abort();
    queue.stop();
    await expect(flush).rejects.toMatchObject({ name: 'AbortError' });
    expect(queue.bufferedBytes).toBe(0);
  });

  test('separated ownership prevents stale polls and older lifecycle states from regressing state', () => {
    const renamed = mergeTerminalSession(session(), session('Production shell'), 'rename');
    const stalePoll = mergeTerminalSession(renamed, session('Original shell'), 'poll');
    expect(stalePoll.label).toBe('Production shell');

    const ended = { ...renamed, state: 'closed' as const };
    expect(mergeTerminalSession(ended, session('Original shell', 'open'), 'poll')).toBe(ended);
  });

  test('an ended session reports one polling error and terminates', async () => {
    const poll = vi.fn(async () => Promise.reject(new Error('removed')));
    const onError = vi.fn();
    await runTerminalPollLoop({
      getSession: () => session('Ended', 'closed'),
      poll,
      terminal: { write: vi.fn() },
      onSession: vi.fn(),
      onError,
      signal: new AbortController().signal,
      wait: async () => {},
    });
    expect(poll).toHaveBeenCalledTimes(1);
    expect(onError).toHaveBeenCalledTimes(1);
  });
});
