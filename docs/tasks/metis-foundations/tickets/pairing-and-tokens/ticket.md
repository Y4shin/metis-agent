---
type: ticket
subtype: feature
title: Pairing, per-surface tokens, and client credential storage
status: stable
workflow_state: ready
blocked_by: [local-identity-and-acl, state-store]
size: m
---

## What to build

The credential lifecycle: a surface pairs once with a preshared key, receives its
own token, stores it safely, and can be revoked on its own without disturbing
other surfaces.

## Acceptance criteria

- [ ] The pairing call is gated by a preshared key resolved from configuration as
      a literal, an environment reference, a keystore reference, or a secret-file
      reference, and a secret file readable by group or others is refused with a
      clear error.
- [ ] A successful pairing mints an opaque per-surface token, the daemon stores
      only its hash, and the token is returned once.
- [ ] The credential travels as authorization metadata on every call including
      the event stream, and never appears in a URL.
- [ ] Missing credentials and invalid or revoked credentials produce
      distinguishable typed errors, and the client maps them so a caller can tell
      pairing is needed from access being gone.
- [ ] Removing one token entry and restarting revokes that surface while the
      other surfaces keep working, asserted by a test.
- [ ] The client keeps the credential in the platform keychain when its command
      line tool is available, falls back to an owner-only file, and reports the
      fallback.
- [ ] A paired client reaches the full exposed surface, with no per-client
      permission filtering.

## Blocked by

- `local-identity-and-acl`, because pairing is an authenticated call on the local
  transport and needs the identity handshake.
- `state-store`, because the daemon keeps token hashes in the SQLite store that
  ticket lands, and this ticket adds no file format of its own. The preshared key
  travels in the pairing request body, and the token is kept by the client, never
  in the store.
