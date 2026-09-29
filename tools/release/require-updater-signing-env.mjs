import { requireUpdaterSigningEnvironment } from './signing-environment.mjs';

try {
  requireUpdaterSigningEnvironment();
  console.log('Updater signing credentials are present');
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
