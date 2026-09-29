import { appendFile, writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { REPOSITORY_ROOT, assertOrdinaryPath, verifyReleaseSource } from './release-common.mjs';
import { verifyTauriGeneration } from './tauri-generation.mjs';
import { verifyUpdaterPublicKey } from './updater-public-key.mjs';

function fail(message) {
  throw new Error(message);
}

export async function prepareUpdaterBundleConfig(
  path,
  root = REPOSITORY_ROOT,
  environment = process.env,
) {
  if (!isAbsolute(path)) fail('bundle config path must be absolute');
  verifyReleaseSource(root, environment);
  await verifyTauriGeneration(root);
  const { encodedPublicKey } = await verifyUpdaterPublicKey(root);
  const output = resolve(path);
  await assertOrdinaryPath(dirname(output), 'directory');
  const config = { plugins: { updater: { pubkey: encodedPublicKey } } };
  await writeFile(output, `${JSON.stringify(config)}\n`, { encoding: 'utf8', flag: 'wx' });
  await assertOrdinaryPath(output, 'file');
  return output;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 4 || process.argv[2] !== '--output')
      fail('expected exactly --output <absolute-path>');
    const path = await prepareUpdaterBundleConfig(process.argv[3]);
    if (process.env.GITHUB_OUTPUT) {
      if (/[\r\n]/.test(path)) fail('workflow output path contains a newline');
      await appendFile(process.env.GITHUB_OUTPUT, `config_path=${path}\n`);
    }
    console.log('Prepared temporary public-key-only bundling configuration');
  } catch (error) {
    console.error(`Updater bundling configuration failed: ${error.message}`);
    process.exitCode = 1;
  }
}
