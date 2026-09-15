# ADR 0001: Rust services behind a narrow Tauri API

Status: Accepted

Use a Cargo workspace with model, core, SSH, secrets, discovery, operations and audit crates. Tauri is a thin composition and IPC layer. React uses a typed application client and never receives a shell or arbitrary command API. Domain models contain no SSH library types. Transport sessions expose an allowlisted read-only command enum internally. Provider interfaces permit future non-SSH transports without adding speculative implementations.

The TypeScript protocol is generated from Rust serializable domain types. UI primitives and tokens are shared in a small package. TanStack Query owns server state and Zustand owns host selection only.
