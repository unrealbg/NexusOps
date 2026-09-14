# ADR 0002: Explicit TOFU and OS credential storage

Status: Accepted

SSH uses russh with real server-key verification before authentication. Unknown keys abort the connection and produce an explicit trust challenge. Trust is bound to a pending challenge and canonical endpoint. A trusted endpoint cannot be overwritten through the trust API. A changed key blocks connection; rotation requires deliberate out-of-band verification and local trust maintenance.

Host metadata and host-key pins are local SQLite data. Credentials are separate OS secure-store entries and never have a plaintext fallback. Sensitive Rust input is not Debug/Serialize and is zeroized where practical. Frontend secret fields exist only transiently in the host form and IPC payload and are cleared after submission or dismissal.

Local account compromise can expose application secrets. This is outside the keychain's protection boundary; the threat model documents it.
