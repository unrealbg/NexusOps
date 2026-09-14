import type { HostInput } from '@nexusops/protocol';

const hasControlCharacter = (value: string) =>
  [...value].some((character) => character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127);

/** Immediate form feedback; Rust independently validates every field at the boundary. */
export function validateHost(input: HostInput): string | null {
  if (!input.displayName.trim()) return 'Enter a display name.';
  if (input.displayName.length > 100) return 'Use a display name of 100 characters or fewer.';
  if (
    !input.connection.hostname.trim() ||
    /[\s/]/.test(input.connection.hostname) ||
    hasControlCharacter(input.connection.hostname)
  )
    return 'Enter a hostname or IP address without spaces or a protocol prefix.';
  if (
    !Number.isInteger(input.connection.port) ||
    input.connection.port < 1 ||
    input.connection.port > 65535
  )
    return 'Enter an SSH port between 1 and 65535.';
  if (
    !input.connection.username.trim() ||
    /\s/.test(input.connection.username) ||
    hasControlCharacter(input.connection.username)
  )
    return 'Enter an SSH username without spaces.';
  return null;
}
