import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { REPOSITORY_ROOT, assertOrdinaryPath } from './release-common.mjs';

export const UPDATER_PUBLIC_KEY = 'docs/release/keys/nexusops-updater.pub';
const MAX_PUBLIC_KEY_BYTES = 1024;

function fail(message) {
  throw new Error(message);
}

function canonicalBase64(text, label) {
  if (!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(text))
    fail(`${label} is not canonical base64`);
  const decoded = Buffer.from(text, 'base64');
  if (decoded.toString('base64') !== text) fail(`${label} is not canonical base64`);
  return decoded;
}

export function validateUpdaterPublicKey(bytes) {
  if (!Buffer.isBuffer(bytes) || bytes.length === 0 || bytes.length > MAX_PUBLIC_KEY_BYTES)
    fail('updater public key size is invalid');
  const encoded = bytes.toString('utf8');
  if (!Buffer.from(encoded, 'utf8').equals(bytes) || /[\x00-\x20\x7f]/.test(encoded))
    fail('updater public key contains invalid text');
  const decodedBytes = canonicalBase64(encoded, 'updater public key');
  const decoded = decodedBytes.toString('utf8');
  if (!Buffer.from(decoded, 'utf8').equals(decodedBytes) || /[\x00-\x08\x0b-\x1f\x7f]/.test(decoded))
    fail('decoded updater public key contains invalid text');
  const match =
    /^untrusted comment: minisign public key: ([0-9A-F]{16})\n([A-Za-z0-9+/]{56})\n?$/.exec(
      decoded,
    );
  if (!match) fail('updater public key is not one minisign public key');
  const key = canonicalBase64(match[2], 'minisign public key');
  if (key.length !== 42 || key.toString('ascii', 0, 2) !== 'Ed')
    fail('updater public key has invalid minisign key bytes');
  const keyId = Buffer.from(key.subarray(2, 10)).reverse().toString('hex').toUpperCase();
  if (keyId !== match[1]) fail('updater public key identifier differs from key bytes');
  return {
    sha256: createHash('sha256').update(bytes).digest('hex'),
    encodedPublicKey: encoded,
  };
}

export async function verifyUpdaterPublicKey(root = REPOSITORY_ROOT) {
  const path = join(root, UPDATER_PUBLIC_KEY);
  await assertOrdinaryPath(path, 'file');
  return validateUpdaterPublicKey(await readFile(path));
}
