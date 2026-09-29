import { createHash } from 'node:crypto';

const MAX_SIGNATURE_BYTES = 4096;

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

export function inspectUpdaterSignature(bytes, artifactBasename, productVersion) {
  if (!Buffer.isBuffer(bytes) || bytes.length === 0 || bytes.length > MAX_SIGNATURE_BYTES)
    fail('updater signature size is invalid');
  if (
    typeof artifactBasename !== 'string' ||
    !/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(artifactBasename) ||
    typeof productVersion !== 'string' ||
    !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(productVersion)
  ) {
    fail('updater signature expectation is invalid');
  }
  const encoded = bytes.toString('utf8');
  if (!Buffer.from(encoded, 'utf8').equals(bytes) || /[\x00-\x20\x7f]/.test(encoded))
    fail('updater signature contains invalid text');
  const decodedBytes = canonicalBase64(encoded, 'updater signature');
  const decoded = decodedBytes.toString('utf8');
  if (!Buffer.from(decoded, 'utf8').equals(decodedBytes) || /[\x00-\x08\x0b-\x1f\x7f]/.test(decoded))
    fail('decoded updater signature contains invalid text');
  const lines = decoded.endsWith('\n') ? decoded.slice(0, -1).split('\n') : decoded.split('\n');
  if (lines.length !== 4 || lines[0] !== 'untrusted comment: signature from tauri secret key')
    fail('updater signature has malformed minisign lines');
  const signedBytes = canonicalBase64(lines[1], 'minisign signature');
  if (
    signedBytes.length !== 74 ||
    !['Ed', 'ED'].includes(signedBytes.toString('ascii', 0, 2))
  ) {
    fail('updater signature has invalid minisign signature bytes');
  }
  const trusted = /^trusted comment: timestamp:([0-9]{1,16})\tfile:([^\t]+)\tversion:([^\t]+)$/.exec(
    lines[2],
  );
  if (!trusted || trusted[2] !== artifactBasename || trusted[3] !== productVersion)
    fail('updater signature trusted comment lacks exact artifact/version binding');
  if (!Number.isSafeInteger(Number(trusted[1])) || Number(trusted[1]) <= 0)
    fail('updater signature timestamp is invalid');
  const globalSignature = canonicalBase64(lines[3], 'minisign global signature');
  if (globalSignature.length !== 64) fail('updater signature has invalid global signature bytes');
  return {
    sha256: createHash('sha256').update(bytes).digest('hex'),
    artifactBasename,
    productVersion,
  };
}
