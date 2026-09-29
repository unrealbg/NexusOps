export function requireUpdaterSigningEnvironment(environment = process.env) {
  if (
    typeof environment.TAURI_SIGNING_PRIVATE_KEY !== 'string' ||
    environment.TAURI_SIGNING_PRIVATE_KEY.length === 0 ||
    typeof environment.TAURI_SIGNING_PRIVATE_KEY_PASSWORD !== 'string' ||
    environment.TAURI_SIGNING_PRIVATE_KEY_PASSWORD.length === 0
  ) {
    throw new Error('updater signing credentials are unavailable');
  }
}
