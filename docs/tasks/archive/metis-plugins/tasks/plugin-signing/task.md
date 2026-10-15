---
type: task
subtype: grilling
title: Bundle signing scheme and trust-chain policy
status: deprecated
workflow_state: done
blocked_by:
- plugin-signing-format
mode: human
---

## Decision to settle

The signing and trust policy a `.metis` bundle is held to:

- the signature scheme and what exactly it covers, whether the whole archive
  or a digest of the manifest and the libraries;
- where the signature lives, as a detached file beside the `.metis` or as an
  entry and comment inside the ZIP;
- the trust chain: a flat list of trusted public keys, a root key that signs
  plugin keys, or platform trust stores, and how many links the verification
  walks;
- how a key is identified in the manifest or beside the signature, and what
  happens when the identifier is unknown;
- how trust is withdrawn: a revocation list, removal of the key from the
  trusted set, or an expiry;
- who supplies the trust policy, given that the runtime never reads the
  daemon's configuration files;
- when verification happens, before extraction or after, and what refusal
  looks like to a caller;
- whether an unsigned bundle can be loaded at all, and if so under what
  switch, if any.

## Parent decisions

Already settled and not to be reopened:

- A bundle is a ZIP archive with the extension `.metis`, and extraction
  refuses absolute or `..` names, symlink and hardlink entries, permissions,
  and oversized entries.
- The manifest is `manifest.toml` at the bundle root and declares the
  capability list, which is cross-checked against `list_capabilities` on load.
- The runtime never reads the daemon's configuration; the daemon hands it
  what it needs.
- Native plugins run in process and can crash it. A signature says who built a
  bundle; it does not make the plugin safe to run.

## Choices already known

- The effort already takes one archive dependency for the bundle. The
  signature library is a second one, and the research findings record its
  transitive cost.
- The bundle is a single file, so a detached signature is one more file to
  ship, while an embedded signature keeps the artifact single.
- The manifest is the natural place for the signing key's identifier, since
  the host reads it before it maps any library.
- The daemon resolves secrets and configuration, so the trusted key set
  arrives as host input rather than as something the runtime discovers.
- The C fixture hand-writes its manifest, so any field the signing scheme adds
  to the manifest must remain hand-writable.

## Recommended starting answer

Superseded in part by the decisions recorded at the end of this file: the
operator chose a single file rather than a detached sidecar, and a root key set
with a revocation list rather than a flat trusted key set. The Ed25519 part
stands.

Ed25519 over the whole `.metis` file, with the signature as a detached file
beside the bundle, and a trust chain of one flat level: a set of trusted public
keys supplied by the daemon, each with a key identifier that the manifest
names. Verification happens before extraction,
an unknown or untrusted key refuses the bundle outright, and there is no
unsigned path and no online verification in v1. The research findings may
overturn this, and if they do, record why.

## Dependent-task implications

- `compatibility-policy` decides what an unsigned or untrusted bundle does
  relative to a version mismatch, and whether refusal is a manifest error, a
  load error, or both.
- The bundle and manifest tickets carry the signature file, the manifest's key
  field, and the verification gate before extraction.
- `configuration-and-host-context` carries the trusted key set as host input,
  which is where the daemon supplies it.
- `metis-daemon` owns the configuration that names the trusted keys, and
  `metis-plugin-sdk` owns the CLI that signs a bundle.

## Remaining fog

- Where trust roots are stored at rest, and in what format.
- Whether a signature is required for a local development load, and how such a
  load is kept out of the installed path.
- Whether the manifest names one key or a chain, once the research findings are
  in.

## Completion evidence

The decision recorded in this document, with the alternatives considered and
the reason each was rejected, plus the dependent-task implications above.

## Decisions recorded

S-numbers are this task's ledger, separate from the one in
`configuration-and-host-context`.

**S1, one file with a container header.** The bundle stays a single file. A
versioned header region carries the signature, and the ZIP bytes follow it. The
header carries at least the archive format version, the algorithm identifiers,
the key id, the signer's certificate chain and the signature. The bundle format
version moves here from the manifest, because it has to be readable before the
archive can be parsed. The operator delegated the layout, so this is the choice:
a fixed-size prefix header, then the chain, then the signature, then the ZIP
bytes; the signed range is the header, the chain and the ZIP bytes, with the
signature excluded by the offset and length the header declares. Downloading two
files for a signed plugin was the reason the detached sidecar was rejected.

**S2, root keys plus a revocation list, and a certificate chain inside the
bundle.** The daemon holds a set of trusted root keys (a set, so two can be
active while a rotation overlaps) and a revocation list. Both are compiled into
the daemon by default and both are modified through the configuration file. A
bundle carries a certificate chain from its signing key up to a root.
Verification means the leaf key verifies the payload signature, every link
verifies up to a root in the set, and no revoked key appears anywhere in the
chain.

**S3, an unverifiable or unsigned bundle aborts startup.** The operator's reason
is that it is a sign that the machine has been tampered with. This conflicts
with the start-up policy recorded in `configuration-and-host-context`, where a
failed signature degrades that plugin and the daemon keeps running. The conflict
is open, is the first question of the next round, and either outcome needs an
amendment note in that task.

**S4, the signing job holds the key and the chain.** Whoever signs has the
signing key plus the certificate chain up to its certificate, so the bundle can
carry the full chain to the root.

**S5, the trust policy is a struct.** The daemon-facing loader takes a trust
policy struct holding the root public keys and the revocation list, filled from
the compiled defaults plus the configuration file's additions and removals.

Open after this round: the S3 conflict, how a chain link is represented, what a
link binds, what revocation names and whether anything expires, and what the
compiled defaults are in this effort.

## Decisions recorded, second round

**S6, a failed or absent signature is fatal.** The operator changed their mind on
the earlier start-up policy: an unverifiable or unsigned bundle crashes the
daemon instead of degrading that one plugin, because tampering with the machine
is the signal. `configuration-and-host-context` now carries a later amendment
recording this, and its entry about the five bundle-level failures is superseded
on the signature case only. The other four still degrade.

**S7, the key id is derived from the public key.** Revocation names key ids, and
a key id has to be recomputable from the key itself rather than generated at
random as minisign does it, otherwise relabelling a key sidesteps the revocation
list. The choice is a truncated hash of the public key, for example the first 16
bytes of SHA-256 over its encoded form.

**S8, what a chain link carries.** A link is a signature by the parent key over
the child's public key, an arbitrary description field of bounded length, and an
optional expiry in Unix UTC milliseconds. The last signature in the chain is the
payload signature, where the leaf key signs the header, the chain and the ZIP
bytes. The operator prefers a free description to a structured identity, so the
verifier cannot check that a description names the plugin it claims to be, and
replay across plugin names is prevented only by how the root issues keys.

**S9, the compiled root set is empty and the tests carry their own.** No
production root exists yet. Tests build a trust policy holding a test root and
sign the fixture with a matching test key, so the real verification path runs in
CI. The operator adds their own root to the compiled set at release time, so
distributions can load first-party plugins.

**S10, the chain and signature format is deferred.** The operator wants an
established format that is as lightweight as possible, with either a Rust
implementation or a parser simple enough to reimplement safely, because parsing
untrusted input whose acceptance leads to loading compiled code is security
relevant. The `plugin-signing-format` research task answers which format, and
this task is blocked on it.

Open after the second round: whether an expired chain aborts like a failed
signature or degrades, given that a skewed host clock would then refuse a valid
plugin, and whether the verifier checks the description against the plugin's
name or treats it as informational only.

## Decisions recorded, third round

**S11, validity semantics follow the chosen format, and the signature carries its
signing time.** The operator is not sure yet and wants the signature to carry the
timestamp it was made at, and then to either handle validity the way TLS
certificate paths do or to follow whatever the chosen format's specification
says. That makes the rule depend on the format the `plugin-signing-format` survey
names, so it is deferred to the round that follows that survey. What stays open
is what an out-of-validity chain does at start-up: given that a failed signature
is fatal, the consistent answer is that it aborts, but that is not settled.

**S12, the identity check applies to the link that authorises the plugin's own
key.** The chain may contain intermediate links that are not plugin specific: a
developer's master key, signed by a root because that developer is trusted, can
authorise the leaf keys of several plugins. So every link carries a description,
the link whose subject is the leaf key carries the plugin identity, meaning its
name and optionally a version range, and the verifier checks that one against the
manifest. Intermediate descriptions stay informational. This corrects the earlier
note, which assumed every link was plugin specific.

**S13, the format is the minimal project format.** The chain and the payload
signature use the layout the `plugin-signing-format` survey specifies: fixed
stride link records with a domain separated Ed25519 signature each, key ids
recomputed from the public key and never read from the wire, refusal rules that
are a fixed offset walk, and `ed25519-dalek` in verify only configuration at 18
measured third party packages with no C build in the trusted path. Every
established candidate was rejected: each one fails to carry at least one of the
three required fields without a project defined profile, and the validators that
are usable are large, experimental, or hide a C build.

**S14, an expired link is fatal, with a documented skew tolerance.** A few
minutes of tolerance, as TLS allows, so two integrity failures, a bad signature
and an expired link, do not have different outcomes.

**S15, a chain may hold up to eight links, and v1 exercises one or two.** The
walk is the same either way and the cap is a refusal rule, so the general shape
is accepted.

**S16, the domain tags stay version stable and the signature offset is declared.**
The `v1` in `metis-plugin-chain-link-v1` and `metis-plugin-payload-v1` is the
signing input's version, not the bundle format version, so a container format
change does not invalidate every existing signature. The payload signature's
offset is declared in the header.

**S17, the SDK CLI accepts both a pre-signed chain and a parent key.** It takes
the leaf signing key plus either a chain file of already signed parent links or a
parent key it signs the leaf's link with, preferring the chain file, because
parent keys are normally offline while a developer who holds their own master key
can sign locally.

**S18, no name scoping on an intermediate.** Every certification may authorise
any plugin name. The operator's reason: a prefix restriction would not bound a
hostile developer, who would rename the same code, so trust in a certified key is
implicit and the restriction would be theatre. The leaf-authorising link still
names the plugin and is still checked against the manifest.

## Completion record

**The decision in the operator's terms.** A plugin bundle is one file. It begins
with a versioned header carrying at least the archive format version, the
algorithm identifiers, the key id, the signer's certificate chain and the
signature, and the ZIP bytes follow it. The signed range is the header, the chain
and the ZIP bytes, with the signature occupying a gap the header declares.
Verification is raw Ed25519 over a fixed stride link record that this project
defines, because no established format carries a parent signature over the
child's key, an arbitrary bounded description and an optional millisecond expiry
together without a project profile, and the validators that are usable are
large, experimental, or hide a C build. The daemon holds a set of root keys and a
revocation list, both compiled in by default and both editable through the
configuration file; the bundle carries the chain from its signing key to a root;
key ids are a truncated hash of the public key, recomputed by the verifier and
never read from the wire; every link carries a description and an optional
expiry; the link that authorises the leaf names the plugin and is checked against
the manifest; and intermediate descriptions are informational, with any
certification free to authorise any name because trust in a certified key is
implicit. A failed or absent signature, an expired link, or any refusal rule
violation is fatal and the daemon refuses to start. The compiled root set is
empty, the tests carry a test root of their own so CI exercises the real path,
and the operator's root joins the compiled set at release time.

**Alternatives considered and rejected.**

- A detached sidecar signature beside the bundle. Rejected: enforcing a signature
  by default should not require shipping two files.
- X.509, CMS, JOSE with `x5c`, and COSE with `x5chain`. Rejected: each needs a
  project profile for the description or the expiry granularity, the usable
  validators are large, experimental or carry a C build, and the small ones
  cannot expose the fields the chain must carry.
- OpenPGP through `pgp` or `sequoia-openpgp`. Rejected: 146 and 71 measured
  third-party packages, a bindgen and nettle C build for the second, seconds
  resolution expiry, and an expiration and revocation semantics layer the RFC
  itself leaves unspecified.
- minisign or signify once per link. Rejected: an 8 byte random key id defeats a
  revocation list that names recomputable ids, and there is no expiry field.
- PASETO and TUF. Rejected: PASETO has no chain primitive, and TUF is a
  repository permission model with a seconds resolution expiry and a C build in
  its client.
- A flat trusted key set with no chain. Rejected earlier: it makes the operator
  edit the policy for every plugin key change.
- Random key ids. Rejected: a revocation list must name something the verifier
  can recompute from the key.
- An expired link degrading one plugin instead of aborting. Rejected: two
  integrity failures should not have two different outcomes.
- Scoping an intermediate's authority by name prefix. Rejected: a hostile
  developer would rename the same code, so it buys no security.
- Folding the bundle format version into the signing domain tags. Rejected: it
  would invalidate every existing signature on a container change.

**Constraints and rationale.** The format is a project format because the three
required link fields are a conjunction no established format native carries, and
because this project must be able to review or reimplement the parser that reads
untrusted bytes whose acceptance leads to loading compiled code. A C build step
is kept out of the trusted parse path, which is why the two small
certificate-validating crates were refused. **S19 supersedes the format choice in
this paragraph.** The chain is X.509, read by `x509-cert` and `der` with no C
build in the trusted path, and what survives is the requirement that the parser
reading bytes whose acceptance leads to loading compiled code be reviewable. Verification never reads a value it
can recompute, which is why ids are derived and the header's key id is ignored in
favour of the recomputed one. The container layout keeps one file, because
signature enforcement is the default.

**Dependent-task implications.**

- `compatibility-policy` is unblocked and inherits: a signature failure or an
expired link is fatal, an ABI or bundle format mismatch disables the plugin, and
`abi_minor` moves because the table changes.
- `metis-plugin-sdk` owns the CLI, which accepts the leaf key plus either a
pre-signed chain file or a parent key, signs with an Ed25519 crate on the SDK and
build side, and writes the header the runtime verifies. The signing-side crate
choice is still open.
- `metis-daemon` owns the configuration file entries for the root set and the
revocation list, and supplies both through the trust policy struct the loader
takes.
- `plugin-runtime` owns the verifier, the trust policy struct, the recomputed key
ids, and the refusal rules, all of which belong in `docs/plugin-contract.md`.
- The `plugin-contract` amendments this effort owes now also cover the container
header, the link record, the signing input domain tags, and the two key-id and
revocation rules.

**The layout.** **S19 supersedes the link record, the chain and the link signing
input in section 3 of
`docs/tasks/metis-plugins/tasks/plugin-signing-format/findings.md`**: a link is an
X.509 certificate, so the chain region holds concatenated DER certificates. The
payload signing input, the key id rule and the container refusal rules in that
section still apply. The
container header is the one part of the layout this task owns rather than the
survey; the proposal is a fixed 32 byte big-endian header carrying the magic
`METISBND`, the container format version, the suite, the header length, the chain
length, the signature offset and length, a reserved field that must be zero, and
the ZIP length, with the file length required to equal signature offset plus 64
plus ZIP length. `to-spec` pins it.

**Remaining fog, all spec level.**

- The header's exact field sizes, offsets and magic value, and the exact stride
the survey's layout implies, which `to-spec` writes down.
- The configuration file's representation of root keys and revocation entries.
- The signing-side Ed25519 crate for the SDK CLI: `ed25519-dalek` with its
  signing features measured 21 third-party packages, `ed25519-compact` 5.
- Whether a committed test vector is required, which the survey recommends.
- Minisign is now out of the design entirely: the payload signature is raw
  Ed25519, so the earlier scheme recommendation's minisign wire format is
  superseded by S13 even though the Ed25519 choice stands.

**Completion evidence.** The eighteen recorded decisions, the rejected
alternatives, the constraints and rationale, the dependent-task implications and
the remaining fog, confirmed by the operator.

## Format change, fourth round

**S19, the chain is X.509 with a project profile, not a project format.** The
operator accepted the reversal after the field mapping: every field the chain
needs has an X.509 home, the millisecond expiry was this effort's own invention
and no consumer needs it, and X.509 brings standard issuance tooling, so the SDK
CLI does not have to issue the chain. S13 is superseded, and the survey's
rejection of X.509 is overridden.

Consequences, which replace the corresponding parts of the completion record
above:

- A link is an X.509 v3 certificate with an Ed25519 key, per RFC 8410. The child's
  key is `subjectPublicKeyInfo`, the parent's signature is over `tbsCertificate`,
  and the chaining rule is that the child's `issuer` equals the parent's
  `subject`.
- The description is the subject `CN`, a directory string. The
  leaf-authorising certificate's `CN` is the plugin name and is checked against
  the manifest, and intermediate subjects stay informational under S18.
- Validity is `notBefore` and `notAfter` at second granularity and is mandatory
  rather than optional, so the `expiry_present` and `expiry_millis` fields
  disappear. An out-of-window certificate is fatal with the skew tolerance of
  S14.
- The profile, which X.509 does not supply: a certificate that authorises another
  key must carry `basicConstraints` with `CA:TRUE`, the leaf-authorising
  certificate must be an end entity, and any critical extension the verifier does
  not understand refuses the chain.
- Key ids stay recomputed, `SHA-256(public key)[0..16]`, and `subjectKeyIdentifier`
  is ignored, which RFC 5280 expressly permits. Revocation stays this project's
  own local list of those ids, not `CRL` or `OCSP`, which are network and time
  oriented.
- Parsing uses `x509-cert`, measured at 13 third-party packages with no C build,
  over `der` and `ed25519-dalek` verify-only, and the path check for one to three
  links is this project's. The strict DER decoder is what makes re-encoding
  `tbsCertificate` safe, because a non-canonical encoding is refused before it is
  interpreted.
- The container header, the payload signature and its domain tag stay this
  project's, because X.509 certifies keys and does not sign this bundle: the leaf
  key still signs the header, the chain and the ZIP bytes with raw Ed25519. The
  `metis-plugin-chain-link-v1` tag disappears with the link record, and
  `metis-plugin-payload-v1` stays.
- The header's chain region is a concatenation of DER certificates, which are
  self-delimiting, so the header declares only the region length and the verifier
  requires it to be consumed exactly, with no trailing bytes.

Open after the change: the profile text, meaning which `basicConstraints` and
`keyUsage` requirements apply to which link, the issuance tooling choice between
`openssl` and `rcgen` and the signing-side dependency cost that follows, and the
header's exact field list.
