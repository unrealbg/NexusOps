# Goal 02C source validation

Base: `c3939415b5980095f0acef3f60e2d4fe35c00180` (`main` after Goal 02B merge). This report covers source and disposable OpenSSH validation only. **NATIVE ACCEPTANCE PENDING.** No NexusOps native app was launched.

The editor accepts one regular non-symlink SFTP v3 file up to 1 MiB as strict UTF-8. It preserves BOM, LF/CRLF and final-newline presence; mixed newlines, NUL and invalid UTF-8 fail closed. A server-held document token binds exact host/session/path metadata and original-byte SHA-256 to a one-shot save plan. Saves use the existing transfer queue, exclusive same-directory staging, owned cleanup and POSIX rename. Mode and uid/gid are reapplied and verified before replacement. The renderer retains dirty text across disconnect or conflict but cannot rebind old authority.

Local validation (Windows MSVC, locked Rust dependencies, repository npm lockfile):

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS; upstream `ts-rs` serde-transparent parser notices |
| `cargo test --workspace --lib --bins --locked` | PASS |
| `cargo test -p nexus-ssh --test loopback --locked` | PASS, 9 tests |
| `cargo run -p nexus-core --example export_protocol --locked -- --check` | PASS |
| `npm run typecheck` | PASS |
| `npm run lint` | PASS |
| `npm test` | PASS, 9 files / 81 tests; jsdom canvas notice |
| `npm run build` | PASS; Vite large-chunk warning |

The disposable real OpenSSH fixture passed `openssh_phase_a`, terminal PTY interoperability, the existing 256 MiB SFTP streaming scenario, key rotation rejection, and the new editor interoperability scenario. The editor test verified LF, CRLF, BOM, exact reopened bytes, mode `0644`, and Conflict after an independent remote replacement. A separate deterministic pause after the first real staging write verified that disconnect cancels the editor job, removes the confirmed-owned `.edit.part` staging file before SFTP close, and leaves the original bytes unchanged. All fixture distributions and scratch data were cleaned through the marker-validated script. No production host or private file was used.

Known limits: SFTP v3 metadata plus content digest and `posix-rename@openssh.com` is not atomic CAS against a writer acting after the final check. ACLs, xattrs, SELinux labels, capabilities, hard-link identity and other metadata not exposed by SFTP v3 cannot be preserved or verified. A lost final replacement reply remains `OutcomeUnknown` without replay. Owner native Windows acceptance is pending source review; see [editor architecture and checklist](../architecture/editor.md).
