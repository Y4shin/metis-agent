---
type: deviation report
title: Deviation report for bundle-verification
status: stable
---

## Deviation report: bundle-verification

Compared: `docs/tasks/metis-plugins/arch-spec.md` section `bundle-verification`,
the ticket doc and its acceptance criteria, and the implementation on
`ticket/bundle-verification` (`git diff task/waker-lifecycle-fix..ticket/bundle-verification`;
the landing branch already carried the waker fix). Thirteen files changed, two of
them new test files and one new module.

### API surface changes

**1. The container is fixed exactly as planned, and the domain tag's bytes are
fixed here.**
- **Planned:** a 40-byte big-endian header, magic `METISBND`, version at offset
  8, reserved field zero, file length equal to signature offset plus signature
  length plus ZIP length, signed message over the domain tag, the header, the
  chain and the archive with the signature bytes deleted rather than zero-filled,
  and a domain tag that is version stable and independent of the container
  version, whose bytes the arch spec did not name.
- **Actual:** byte for byte as planned. The tag's bytes are fixed by this ticket
  as the 27 ASCII bytes `METIS-PLUGIN-BUNDLE-PAYLOAD`, exported as `DOMAIN_TAG`
  and written into `docs/plugin-contract.md` section 17.2 as the public constant
  `CONTAINER_MAGIC` and `DOMAIN_TAG`.
- **Impact:** an irreversible format commitment for every producer. The packaging
  step of `sample-bundle-ci` and the `metis-plugin-sdk` signing CLI must sign
  `DOMAIN_TAG || header || chain || archive`. The tag carries no container
  version, so a later container version bump does not change what a signature
  covers.

**2. The anchor is the chain's last link, trusted by recomputed key id.**
- **Planned:** `TrustStore` carries "root keys", and an unknown root refuses the
  chain. The arch spec did not say how a chain reaches a root key.
- **Actual:** the chain carries the anchor's own certificate as its last link,
  and the anchor is trusted by its recomputed key id rather than by who signed
  it. The root set stays a set of public keys. Written into contract 17.3 and
  17.4; enforced by `check_anchor`.
- **Impact:** the packaging side of `sample-bundle-ci` and the SDK CLI must
  include the root's certificate as the last chain link; a chain that omits it is
  refused as `UnknownRoot`. Revoking a root key refuses every chain anchored at
  it. `TrustStore` stays constructible from plain public keys, which is what
  `startup-policy` needs.

**3. Public surface is wider than the plan, additively.**
- **Planned:** `TrustStore`, `BundleHeader`, `VerifiedBundle`, `BundleError`,
  `verify_bundle`, and the classification predicate `is_trust_failure()`. The
  arch spec named the error variants' families, not their exact set.
- **Actual:** all planned items exist with the planned names and shapes
  (`verify_bundle(bytes: &[u8], trust: &TrustStore) -> Result<VerifiedBundle<'_>, BundleError>`;
  `VerifiedBundle::{header, archive, leaf_common_name}`). Added alongside them:
  the constants `CONTAINER_MAGIC`, `CONTAINER_VERSION`, `SIGNING_SUITE`,
  `HEADER_LENGTH`, `MAX_CHAIN_LINKS`, `KEY_ID_LENGTH`, `DOMAIN_TAG`; the type
  `KeyId` and the function `key_id`; `RootKey`;
  `TrustStore::{empty, with_root, add_root, revoke, roots, revoked}`;
  `BundleHeader::parse`; and 26 error variants, which is the planned families
  plus `Truncated`, `UnknownSigningSuite`, `BadHeaderLength`, `ReservedNotZero`,
  `ChainSpanMismatch`, `MalformedSignature`, `SignatureInvalid` and
  `MissingCommonName`.
- **Impact:** additive only, so no dependent is broken. `startup-policy` should
  classify through `is_trust_failure()` rather than match variants, because the
  variant set is wider than the plan named and the predicate is the stable seam.

**4. `VerifiedBundle` does not expose the parsed chain.**
- **Planned:** the arch spec listed the parsed chain as one of the things a
  `VerifiedBundle` carries.
- **Actual:** the chain is a private field. The public surface is `header()`,
  `archive()` and `leaf_common_name()`, so `x509_cert::Certificate` never enters
  this crate's public API.
- **Impact:** none on the planned dependents: `bundle-extraction` needs the
  archive bytes and `manifest-and-library` needs the leaf common name, and both
  are exposed. A later ticket that needs a chain detail must add an accessor
  first, which is the intended gate.

**5. "Bytes before the header" was reinterpreted, because the criterion as
written has no trigger.**
- **Planned:** the ticket's first acceptance criterion lists "bytes before the
  header" as one of the refusals.
- **Actual:** the header begins at offset 0, so no file byte can precede it. The
  implementer read the criterion as a declared signature offset below the
  header's end (`BundleError::BytesBeforeHeader`), checked before the
  offset-equals-header-plus-chain rule because one header can trip both
  (contract 17.6).
- **Impact:** the refusal exists, is typed, and is documented; nothing depends on
  its trigger. The criterion's literal wording is unsatisfiable and should not be
  copied into a later document.

**6. The compiled-in root set is the runtime's empty default.**
- **Planned:** criterion 4 wants a vector signed with a key the daemon's
  compiled-in root set does not hold.
- **Actual:** the check runs against `TrustStore::empty()`, and no daemon wiring
  exists yet, so there is no daemon-side compiled-in set to test against.
- **Impact:** none. `startup-policy` owns the daemon's default; when it lands,
  the claim is still that the runtime trusts nothing until a root is registered.

### Abstraction usage

- Used/was specified: **yes**. The error enum follows `CallError`'s shape, the
  crates were added once to the workspace dependency table with the reason in a
  comment, the test signer sits in the existing `tests/support/` module, and
  `sha2` was reused from the table rather than re-added.
- The do-not list held in full: no hand-rolled DER or ASN.1 (the strict `der`
  decoder owns the refusals), no hand-rolled Ed25519 and no `ring`, `openssl` or
  `rustls-webpki`, no C toolchain in the trusted parse path (the only added crate
  with a build script is `curve25519-dalek`, which selects cfg flags and compiles
  nothing), no ZIP reading, no daemon configuration read, and no ABI change.
- The verifier module is one file, `crates/plugin-runtime/src/bundle.rs` (911
  lines), rather than a `bundle/` directory. The arch spec named the module, not
  its file split, so this is an implementation choice and not a deviation. It is
  worth splitting when a later ticket grows it, and `bundle-extraction` is the
  natural moment.
- The test signer is a deliberate second implementation of the layout, the
  domain tag and the chain profile, which is what the plan asked for: the
  committed vectors are rebuilt from it and compared byte for byte on every run,
  so a changed offset fails the suite instead of passing quietly.

### Out-of-scope changes

- None in source. The changed set is exactly the planned one: `Cargo.toml` and
  `Cargo.lock`, `crates/plugin-runtime/Cargo.toml`, `src/lib.rs` (one `pub mod`
  line), the new `src/bundle.rs`, the new `tests/bundle_verification.rs`, the new
  `tests/support/signer.rs` plus one module line, two committed vectors,
  `docs/plugin-contract.md` (new section 17), `docs/testing.md`, and the ticket's
  own implementation note.
- Two dev-only dependency additions are new information for the reviews:
  `x509-cert`'s `builder` feature and `ed25519-dalek`'s `alloc` and `pkcs8`
  features, in `[dev-dependencies]` only. No shipped build compiles a signing
  path; the measurement excludes them (66 to 85 crates for normal edges, 76 to
  101 including dev).
- `zip` was correctly not added and no extraction code landed, because extraction
  is the next ticket. No loader, no registry change, and no runtime start.
- Working tree: `docs/tasks/state.yaml` is modified by the workflow runner. It
  was left unstaged and is not part of this ticket.

### Ticket doc update needed?

Yes, and it is already there: the tdd-worker wrote an `## Implementation notes`
section recording the dependency measurement, the domain tag bytes, the anchor
rule, the bytes-before-header reading, the non-canonical trigger, and the
signer's second-implementation rule. The land-worker should append its landing
note to that section rather than add a second one. The four format decisions a
dependent needs (domain tag bytes, anchor rule, the understood extension set
limited to `basicConstraints`, and the 60-second validity skew) are all recorded
in `docs/plugin-contract.md` section 17, which is where a dependent should read
them.

### User attention needed?

Yes, one sentence is enough. The container now carries commitments the arch spec
left open and that the SDK effort must match: the domain tag bytes
`METIS-PLUGIN-BUNDLE-PAYLOAD`, the anchor-is-last-link rule (a bundle carries its
own root's certificate), the understood extension set limited to
`basicConstraints` with every other critical extension refused, and 60 seconds of
clock skew. None of them blocks a dependent ticket in this effort; all are
documented in contract 17, and the refusal split for `startup-policy` is now the
`BundleError::is_trust_failure()` predicate.
