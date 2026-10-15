---
type: task
subtype: research
title: Which signature scheme and trust-chain shape fit a native plugin bundle
status: deprecated
workflow_state: done
blocked_by: []
---

## Question

Which signature scheme and trust-chain shape should a `.metis` bundle use, and
what does each candidate cost in dependencies, key handling, and verification
work on Linux, macOS, and Windows?

The question has four coupled parts, because a choice in one constrains the
others:

1. **Scheme.** A detached Ed25519 signature over the bundle bytes, an embedded
   signature entry inside the ZIP, a platform-native signature, or a keyless
   online scheme.
2. **Signed payload.** The whole `.metis` file, or a digest of the manifest and
   each selected library, so a bundle can be re-targeted without re-signing.
3. **Trust chain.** A flat set of trusted public keys, a root key that signs
   plugin keys, or platform trust stores. Include how a key is identified, and
   how trust is withdrawn.
4. **Key custody.** The key file format, whether the private key is encrypted,
   and what a build machine needs to sign.

## Decision this unblocks

It unblocks `plugin-signing` (the operator's decision on the scheme and the
trust policy) and, through it, `compatibility-policy`, which decides what the
host does with an unsigned or untrusted bundle. Until the candidates are
measured, `plugin-signing` would be a preference question with no cost data.

## Trusted source boundaries

- Primary: the specifications and official repositories of each candidate
  (RFC 8032 for Ed25519, the minisign and signify formats, the sigstore
  specification, the ZIP specification's extra-field and comment space), and
  the crate documentation for each Rust implementation at a pinned version.
- Local: `Cargo.toml` and `Cargo.lock` for the current dependency budget,
  `docs/plugin-contract.md`, `docs/tasks/metis-plugins/map.md`,
  `crates/plugin-runtime/Cargo.toml`, and `docs/testing.md` for the gate
  commands.
- The repository's stated preference is to keep dependencies small. A scheme
  whose Rust implementation drags in a TLS stack or an HTTP client is a real
  cost, the same way `jsonschema`'s default features were refused for pulling
  `reqwest` and `rustls`. Measure the transitive dependency count, not just
  the direct crate.

## Evidence required for completion

- A candidate table with, per candidate: what is signed, the key and signature
  formats, the transitive dependency count with the version pinned, whether it
  works offline, whether it works on all three targets, and what verification
  refuses.
- The failure modes: which of a modified manifest, a swapped library, an
  added entry, a removed entry, and a truncated file the scheme detects, and
  which it does not.
- Key distribution and rotation: where a public key lives, how a plugin author
  obtains a key that the daemon already trusts, and what a rotation requires.
- A recommendation with the reason, and the candidates rejected with the
  reason each fails.
- Every material claim cited to a specification, a repository path, or a
  version-pinned crate document.

## Dependent tasks

`plugin-signing` waits on this answer, and `compatibility-policy` waits on
`plugin-signing`. If no candidate can be verified offline on all three
targets, that is itself a decision input for `plugin-signing` and has to be
recorded as such.
