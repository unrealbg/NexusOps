# ADR 0004: Encrypted vault with an OS-stored master key

Status: Accepted

Private keys may exceed Windows Credential Manager's per-entry size limit. Store credential documents in a local AES-256-GCM vault with a random 256-bit master key in the platform secure store. Use random nonces and bind ciphertext to immutable credential revision IDs as associated data. This is platform-backed storage with encrypted local payloads, not plaintext configuration.

Write a new immutable credential revision, commit metadata referring to it, then delete the previous revision. Failed commits clean up the new revision. A crash can leave encrypted orphans; automatic orphan collection is deferred because it is not required for the SSH vertical slice. A profile process lock prevents simultaneous master-key creation by two instances. Credentials are not globally deduplicated or shared between hosts.

Master-key reads never create or replace keys. First use explicitly initializes the OS key only for a vault without a persisted initialization marker. The first credential and marker commit together, and deleting all credentials preserves the marker. A missing master key then fails closed for both reads and writes, including after restart. Existing nonempty vaults acquire the marker when opened. This prevents an unavailable key from silently starting a second encryption identity in the same vault.

This avoids split or truncated large private keys in the OS credential store. It adds a small encryption layer and makes keychain backup/recovery a future design requirement. Loss of the master key is unrecoverable; the application does not silently fall back to plaintext.
