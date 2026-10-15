---
type: task
subtype: research
title: Which established, lightweight format carries the plugin signing chain
status: deprecated
workflow_state: done
blocked_by: []
---

## Question

Which established format should carry the plugin bundle's signing chain and its
payload signature? The daemon holds root public keys and a revocation list, the
bundle carries a chain from its signing key to a root, revocation names key ids
derived from public keys, and each link must be able to assert the child's public
key, an arbitrary description field of bounded length, and an optional expiry in
Unix UTC milliseconds.

Three constraints decide it and they trade against each other:

1. Established, so the format is specified and reviewable rather than invented.
2. Lightweight at parse time, enough that either a small Rust crate does it or
   this project can reimplement it safely. Parsing runs on untrusted input and
   its acceptance leads to loading compiled code, so a complex parser is an
   attack surface.
3. Chain capable, with the fields above, and with key ids a revocation list can
   name.

Candidates to assess without prejudging: X.509 certificates, with RustCrypto
`x509-cert` and `der` for parsing or `rustls-webpki` and `webpki` for
verification; CMS and PKCS#7; JOSE JWS with an `x5c` header; COSE with an
`x5chain` header, for example through `coset`; OpenPGP, through `sequoia-openpgp`
or the lighter `pgp`; minisign or signify used once per link as a chain of key
certifications; PASETO; and TUF metadata. Include the possibility that the answer
is a minimal project format, and if so argue why no established format qualifies.

## Decision this unblocks

`plugin-signing`, which is blocked on this answer for its chain and signature
format. That task has already settled the container layout, the trust policy
input, the revocation rule, what a link carries and that a failed signature is
fatal.

## Trusted source boundaries

- Primary: the specification or RFC for each format, and the version-pinned crate
  documentation and crates.io dependency API for each Rust implementation.
- Local: `docs/plugin-contract.md`, `docs/tasks/metis-plugins/map.md`,
  `docs/tasks/metis-plugins/tasks/plugin-signing/task.md`, and
  `docs/tasks/metis-plugins/tasks/plugin-signing-research/findings.md`, which
  already compares signature schemes and measured their cost. `Cargo.toml` and
  `crates/plugin-runtime/Cargo.toml` show the dependency budget, including the
  comment where `jsonschema`'s default features were refused for pulling
  `reqwest` and `rustls`.
- Transitive dependency counts are measured by the sibling `cargo tree` child, so
  cite direct dependencies here and say the transitive count is measured
  elsewhere.

## Evidence required for completion

- A candidate table per format: does it express a chain, can a link carry an
  arbitrary description and an optional expiry, is a key id derivable from the
  public key, which Rust implementation exists and what its direct dependencies
  are, how much code verification takes, and whether the parser is simple enough
  to review or reimplement.
- For each candidate, what happens on hostile input, and which of these five
  tamper cases it detects: a modified manifest, a swapped library, an added
  entry, a removed entry and a truncated file.
- A recommendation with its reason, and every rejected candidate with the reason
  it fails.
- If the recommendation is a project format, the argument for why every
  established candidate fails, plus the minimal canonical byte layout it should
  use, written so a second implementer could reproduce it.
- Every material claim cited to a specification URL or a repository path.

## Dependent tasks

`plugin-signing` waits for this answer. `metis-plugin-sdk` implements signing
against whatever format it names, and `plugin-runtime` implements verification,
so the layout it recommends becomes contract text in `docs/plugin-contract.md`.
