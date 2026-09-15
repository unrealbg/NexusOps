# OpenSSH interoperability validation

Validated on 2026-09-14 against a disposable WSL2 Linux distribution imported only for Goal 01A. The server was OpenSSH 10.3p1 (`openssh-server-10.3_p1-r1`, OpenSSL 3.5.6) on Alpine Linux 3.24.0, kernel `6.18.33.2-microsoft-standard-WSL2`. NexusOps connected to `127.0.0.1:52668`; port 22 was not used.

This is an engineering interoperability test, not an independent third-party security audit.

## Fixture design

The fixture scripts are in `tools/openssh-fixture/`:

- `Setup-OpenSshFixture.ps1` downloads the official Alpine 3.24.0 x86-64 minirootfs and its published SHA-256 file, verifies the archive, imports a uniquely named WSL2 distribution, chooses a free loopback port, and runs `configure.sh`.
- `configure.sh` installs only Alpine's OpenSSH server and account-management package. It creates `nexusops_fixture`, a non-root account with no sudo package or sudo membership, generates disposable credentials, two Ed25519 host keys, and unencrypted and encrypted Ed25519 client keys.
- `Start-OpenSshFixture.ps1` runs `sshd` only inside that disposable distribution and checks the loopback-forwarded port.
- `Run-OpenSshInterop.ps1` runs the two ignored Rust phases, rotates the real server key, and reruns the changed-key phase.
- `Cleanup-OpenSshFixture.ps1` requires an explicit approved root, a strict owned child, a valid versioned ownership record, no reparse point, and a matching WSL registry identity and installation path before it unregisters the distribution and removes its scratch data.

The rootfs used in this run had SHA-256 `de9a11c0e0e7e9c94db3ed8af7b450eafc0b13687bd7e9199d55050f20aa0a89`. The fixture used OpenSSH defaults for algorithms, except that it supplied only a disposable Ed25519 host key. It did not enable obsolete algorithms or weaken key exchange or cipher settings. Password and public-key authentication were enabled for the test user; root login, empty passwords, forwarding, tunnels, user-controlled environment variables, and X11 forwarding were disabled.

The fixture is local-only in this environment. WSL localhost forwarding exposed the chosen port on Windows loopback. No production server, existing SSH configuration, existing `known_hosts` file, or user private key was used.

## Reproduction

From the repository root in PowerShell:

```powershell
$allowed = "<existing-disposable-root>"
$fixture = & .\tools\openssh-fixture\Setup-OpenSshFixture.ps1 -AllowedRoot $allowed
& .\tools\openssh-fixture\Start-OpenSshFixture.ps1 -AllowedRoot $allowed -RunRoot $fixture.RunRoot
& .\tools\openssh-fixture\Run-OpenSshInterop.ps1 -AllowedRoot $allowed -RunRoot $fixture.RunRoot
& .\tools\openssh-fixture\Cleanup-OpenSshFixture.ps1 -AllowedRoot $allowed -RunRoot $fixture.RunRoot
```

The tests are ignored in the default suite because they install packages in a disposable WSL distribution and require Windows, WSL2, network access to Alpine mirrors, and an unlocked local environment. The tests themselves are `crates/nexus-ssh/tests/openssh_interop.rs`.

## Results

| Scenario | Result | Evidence |
|---|---:|---|
| Correct password | PASS | Production `SshProvider` authenticated to OpenSSH and completed discovery. |
| Incorrect password | PASS | Returned typed `Authentication`; connection was not reported as connected. |
| Ed25519 private key | PASS | OpenSSH accepted the generated unencrypted client key. |
| Encrypted Ed25519 key | PASS | Correct passphrase authenticated successfully. |
| Incorrect key passphrase | PASS | Returned typed `Authentication`; no public-key auth attempt was accepted. |
| Unknown key accepted explicitly | PASS | First connection returned the exact endpoint, algorithm, and SHA-256 challenge; only an explicit trust call persisted it. |
| Unknown key rejected/cancelled | PASS | Pin lookup remained empty and the OpenSSH log suffix contained no accepted authentication or session start. |
| Trusted reconnect | PASS | Reopened `KnownHosts` and a new `SshProvider` connected without a new challenge. |
| Same endpoint, changed host key | PASS | After real `sshd` restart with key B, connection returned `ChangedHostKey`; the original pin remained unchanged and the changed challenge could not overwrite it. |
| Trust persistence after restart | PASS | Provider/known-host database reopen passed; native application restart also reconnected using persisted trust. |
| Non-default port | PASS | All OpenSSH scenarios used port 52668. |
| Real read-only discovery | PASS | Hostname, Alpine OS/version, WSL kernel, architecture, uptime, load, memory, and root filesystem were obtained through the eight fixed probes. |
| Two independent sessions | PASS | Disconnecting one session left the other usable. |
| Disconnect/reconnect | PASS | Explicit disconnect closed the first session; two subsequent connections succeeded. |
| Unavailable/stalled server | PASS | A TCP peer that accepted but never completed SSH negotiation returned `Timeout` in about 15 seconds and before the 17-second assertion bound. |
| Key A shown, key B later | PASS | Native release displayed A; the server changed to B before approval. The follow-up connection was blocked with both fingerprints and no discovery. |

The server log demonstrates protocol ordering: the unknown-key attempt ended before any `Accepted password`, `Accepted publickey`, or session-start record. Discovery began only after host trust and successful authentication. Server-side SSH logging is the only intentional fixture-side write caused by discovery; NexusOps' eight remote commands are fixed read-only commands.

## Supported key scope

Ed25519 is the supported and independently validated client-key type for the Windows alpha. Optional `russh` RSA support is disabled because RustSec RUSTSEC-2023-0071 reports a medium-severity timing-side-channel advisory in its transitive RSA implementation and no fixed upgrade is available. No obsolete algorithm was enabled to widen coverage. ECDSA, DSA, SSH certificates, agent authentication, forwarding, jump hosts, and key rotation through the UI are not claimed as verified support.

## Cleanup

The run used the distribution name `NexusOps-Goal01A-20260914`. Cleanup verified the fixture marker and exact name, terminated and unregistered that distribution, and removed the generated host keys, client keys, passwords, passphrase, pin database, and server logs from the task scratch directory. Existing `Ubuntu` and `docker-desktop` distributions were not modified or removed.

## Goal 02A terminal extension

Goal 02A reran the authentication/trust suite and added the ignored `openssh_terminal_pty_interoperability` test against a newly imported, marker-owned `NexusOps-Goal02A-20260914` distribution. It used Alpine 3.24.0 and `openssh-server-10.3_p1-r1` on loopback port 20196. The fixture-only package set added `ncurses-terminfo-base` and Vim.

The production `SshProvider` passed PTY and shell request acknowledgement, UTF-8 Bulgarian and symbol output, ANSI styles, remote resize reported as 37 rows by 101 columns, two simultaneous isolated PTYs, closing one while the other continued, 2 MiB of generated output, a real `top` alternate-screen interaction, and transport disconnect teardown. The same run passed phase A authentication/trust before the terminal test and phase B changed-key rejection after rotating to its independently generated key B.

```powershell
$allowed = "<existing-disposable-root>"
$fixture = .\tools\openssh-fixture\Setup-OpenSshFixture.ps1 -AllowedRoot $allowed
.\tools\openssh-fixture\Start-OpenSshFixture.ps1 -AllowedRoot $allowed -RunRoot $fixture.RunRoot
.\tools\openssh-fixture\Run-OpenSshInterop.ps1 -AllowedRoot $allowed -RunRoot $fixture.RunRoot
.\tools\openssh-fixture\Cleanup-OpenSshFixture.ps1 -AllowedRoot $allowed -RunRoot $fixture.RunRoot
```

The current scripts use a versioned ownership record and an explicit approved root, with no dependency on the original Codex directory layout or a task-local build environment. Terminal test output and generated secrets are not printed or copied into this repository.
