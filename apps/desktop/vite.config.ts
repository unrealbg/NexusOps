import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';

function xtermFrozenPrototypeCompatibility() {
  const xtermSuffix = '/@xterm/xterm/lib/xterm.mjs';
  const inheritedAssignment = 'o.toString=s;function t(';
  const ownProperty = 'Object.defineProperty(o,"toString",{value:s,writable:!0,configurable:!0});function t(';
  return {
    name: 'xterm-frozen-prototype-compatibility',
    enforce: 'pre' as const,
    transform(source: string, id: string) {
      if (!id.replaceAll('\\', '/').endsWith(xtermSuffix)) return;
      const occurrences = source.split(inheritedAssignment).length - 1;
      if (occurrences !== 1) {
        throw new Error(`Expected one pinned xterm toString assignment, found ${occurrences}.`);
      }
      // Tauri freezes Object.prototype. Define the namespace's own property without
      // assigning through the inherited, non-writable prototype descriptor.
      return source.replace(inheritedAssignment, ownProperty);
    },
  };
}

export default defineConfig({
  root: fileURLToPath(new URL('.', import.meta.url)),
  plugins: [xtermFrozenPrototypeCompatibility(), react()],
  clearScreen: false,
  server: { host: '127.0.0.1', port: 1420, strictPort: true },
  build: { target: 'es2022' },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    clearMocks: true,
    restoreMocks: true,
  },
});
