import { verifyUpdaterPublicKey } from './updater-public-key.mjs';

try {
  const { sha256 } = await verifyUpdaterPublicKey();
  console.log(`Updater public key structure: PASS; SHA-256 ${sha256}`);
} catch (error) {
  console.error(`Updater public key verification failed: ${error.message}`);
  process.exitCode = 1;
}
