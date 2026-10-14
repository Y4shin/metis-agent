---
type: ticket
subtype: feature
title: Verify a bundle's container, chain and payload signature
status: stable
workflow_state: ready
blocked_by: []
size: l
---

## What to build

Any file can be turned into a verified bundle or a typed refusal. The container
header is read with the magic and the container version at fixed offsets; the
version must match this build's exactly; the certificate chain is verified against
a supplied root key set and revocation list with every key id recomputed; the
payload signature is verified over the header, the chain and the archive bytes;
and every documented refusal rule is enforced. The verification crates are chosen
here, with their dependency cost measured and no C build step in the path that
reads untrusted bytes. A committed signing vector is tested at byte level. The
contract documents the container, the chain, the profile, the domain tags, the key
id rule, the revocation rule and the refusal rules.

## Acceptance criteria

- [ ] A valid signed bundle verifies, and each refusal rule answers its own typed
      error: unknown container version, a signature offset or length that does not
      account for the whole file, bytes before the header, trailing bytes in the
      chain region, more than eight links, an expired certificate, a revoked key,
      an unknown root, and a non-canonical encoding.
- [ ] The chain profile is enforced: a certificate that authorises another key
      carries the CA constraint, the leaf-authorising certificate is an end
      entity, an unknown critical extension refuses the chain, and the
      leaf-authorising certificate's common name is returned for the manifest
      check.
- [ ] Key ids are recomputed from the public key and a carried identifier is never
      trusted.
- [ ] A committed vector and altered copies of it are tested at byte level, and
      the vector is signed with a test key that the daemon's compiled-in root set
      does not contain.
- [ ] The contract describes the container header, the chain, the domain tags, the
      key id rule, the revocation rule and the refusal rules well enough for a
      second implementation.
- [ ] The trusted parse path adds no C build step, and the measured dependency
      count is recorded.

## Blocked by

- None (can start immediately).
