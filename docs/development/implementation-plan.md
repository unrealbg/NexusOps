# Goal 01 implementation plan

The initial workspace was empty. NexusOps is a new Rust workspace and npm workspace.

1. Record boundaries and security decisions; establish domain and typed IPC contract.
2. Implement isolated SSH sessions, explicit TOFU, persistent endpoint pins, timeouts and cancellation.
3. Implement platform credential storage and transactional host metadata persistence.
4. Add independently testable read-only probes, capability registry, operation policy and safe audit events.
5. Connect a Tauri 2 application service to a React host management and overview UI.
6. Run unit and local SSH integration tests, frontend checks, Clippy, production builds and desktop startup verification.
7. Document verified behavior, limitations, setup and CI.

## Risks to resolve

- Trust must bind the exact hostname, port, algorithm and fingerprint; stale prompts must not overwrite pins.
- Host edits and cancellation must not allow an old connection task to publish results into a new session.
- Secret storage errors must fail closed; metadata must contain no credential material.
- Untrusted remote output must be bounded, parsed as data and never logged or rendered as HTML.
- A failed optional probe must preserve the usable connection.
- Generated Rust/TypeScript contracts and tests must prevent IPC drift.
- Native OS dependencies and keychains differ; Windows is verified locally and other platforms have CI build coverage.
