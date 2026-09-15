# Frontend boundaries and verification

React calls the typed application client in `apps/desktop/src/api/client.ts`. Host methods map to nine fixed native commands; terminal methods map to seven dedicated PTY commands. There is no SSH library, generic command string, plugin execution, filesystem or shell-exec API in the UI. Rust is the source of truth for the generated protocol package.

TanStack Query caches host metadata and sessions. Sessions are keyed by host ID; switching hosts remounts the overview and its mutations so errors and discovery do not bleed between hosts. Session status polls once a second for the selected host and every three seconds for list entries. Discovery measurements are explicitly refreshed using the read-only refresh action. The footer shows the observation timestamp.

Zustand stores only the selected host ID. Dialog state is local component state. Credentials use uncontrolled form inputs and are cleared before submission and dismissal. Saving credentials deliberately bypasses TanStack mutations: mutation variables would otherwise retain secrets in their cache. Save returns only host metadata, which is subsequently refetched. React and the WebView cannot promise deterministic zeroization of JavaScript strings; the transient IPC payload necessarily lives until the native call resolves.

The shared UI package supplies semantic theme tokens and small primitives. Native HTML dialogs provide focus containment and Escape dismissal. Forms explicitly focus the first field after opening and restore focus when closed. Disabled navigation labels announce that the section is coming soon. Identity verification requires an explicit review and trust action; a changed key has no override control.

## Commands

Use Node 24.15 or newer. From the repository root:

```sh
npm ci
npm run typecheck
npm run lint
npm test
npm run build
npm run tauri -- dev
npm run tauri -- build --no-bundle
```

TypeScript is pinned to 6.0.3 because the current TypeScript ESLint parser supports versions below 6.1. This is the newest mutually compatible stable toolchain, although TypeScript 7 is independently available.

## Deterministic tests

Fourteen tests cover explicit trust, changed-key rejection, dismissal without trust, credential clearing on secure-store failure and dismissal, authentication changes, metadata-only edits retaining credentials, no secrets retained in query/mutation caches or browser storage, isolated host switching, empty and unavailable navigation states, validation, and missing/zero resource metrics.

## Visual test fixture

Run `npm run dev`, then visit `http://127.0.0.1:1420/src/test/visual.html?state=connected`. Supported states are `connected`, `empty`, `list`, `unknown`, and `changed`. This test entry imports deterministic fixtures, carries a conspicuous simulated-data banner, disables saving, and is never imported by the production application. It does not open SSH connections.

Browser visual checks verified the connected overview, first-host empty state, private-key and password dialogs, trust review, changed-key rejection, initial field focus, Escape dismissal and focus restoration. The local captures are UI test fixtures rather than evidence of a live connection and are excluded from source publication. The ordinary browser entry fails closed with a desktop-only message when the native runtime is absent.
