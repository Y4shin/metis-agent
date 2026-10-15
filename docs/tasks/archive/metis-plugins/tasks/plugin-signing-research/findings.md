---
type: findings
title: Signature scheme and trust-chain shape for a .metis bundle
status: stable
---

# Findings: signature scheme and trust-chain shape for a `.metis` bundle

Produced 2026-10-14 for the `plugin-signing-research` wayfinder task. Two
children produced it: one researcher wrote the scheme and trust-chain note
below from primary sources, and one worker measured the dependency counts with
`cargo tree`. The two are merged here, so the source note's statement that
dependency counts are measured elsewhere is resolved by section 11. The raw
dependency measurement stays at `.work/signing-research/deps.md`, and the
merge itself was done by the parent, which is also the only writer of this
file.


# Research: signature scheme and trust-chain shape for a `.metis` bundle

Scope: the scheme, the signed payload, the trust-chain shape, and key custody for
a `.metis` ZIP bundle whose manifest is `manifest.toml` at the archive root, and
which the host verifies against a trust policy passed in as host input, before it
extracts or loads anything.

Two boundaries on this note:

- **Transitive dependency counts are not measured here.** They are measured by the
  sibling task that runs `cargo tree`. This note records only direct dependencies
  where a primary source (crate manifest, crates.io dependency API, docs.rs) states
  them, and it says so at each point.
- **No repo gate was run and no repo file was modified.** Every claim below is
  cited to a specification, an official repository path, or a version-pinned crate
  document.

## 1. Recommendation

**Detached Ed25519 over the complete byte range of the `.metis` file, in minisign
wire format (prehashed `ED` mode), verified offline on the host before any ZIP
parsing or extraction, against a flat trusted key set that the daemon supplies as
host input, keyed by the minisign 8-byte key id.**

Reasons, each grounded:

1. A detached signature over the whole file covers every byte, so all five
   tampering cases fall out of one rule instead of five format-specific rules. No
   coverage definition is needed for "the ZIP metadata", the entry set, or the
   archive comment.
2. Ed25519 is pure computation: RFC 8032 defines the signature as 64 octets and the
   public key as 32 octets, with no certificate, no clock, and no network in the
   verification path (`https://www.rfc-editor.org/rfc/rfc8032.txt`, sections 5.1.6
   and 5.1.7). Offline verification on Linux, macOS, and Windows is therefore a
   property of the library, not of a platform API. **The task's fallback clause
   ("if no candidate can be verified offline on all three targets, record it as a
   decision input") does not trigger.**
3. minisign gives three things a hand-rolled format would have to invent: an
   8-byte key id that identifies the signing key without trying every key, a
   trusted comment that is signed and therefore binds the bundle name and version
   (downgrade protection), and a prehashed mode that verifies a large file as a
   stream rather than in memory (`https://jedisct1.github.io/minisign/`).
4. Authoring tooling already exists on all three platforms and is small: the
   `minisign` Rust crate 0.10.0 is pure Rust with four direct dependencies
   (`ct-codecs`, `getrandom`, `rpassword`, `scrypt`) per the crates.io dependency
   API (`https://crates.io/api/v1/crates/minisign/0.10.0/dependencies`), and the
   host side can use `minisign-verify` 0.3.0, which reports zero dependencies
   (`https://crates.io/api/v1/crates/minisign-verify/0.3.0/dependencies` returns
   an empty array; the crate page says "No external dependencies").
5. The trust-chain shape matches the constraint that the runtime never reads the
   daemon's configuration: the policy is a plain set of key ids and 32-byte public
   keys passed in as host input, so no PKI, no trust store, no transparency log,
   and no clock are involved.

The one design point the recommendation does not settle on its own, because it is
a product decision, is the carrier: a sidecar file (recommended for v1) or an
in-band signature. Section 5 sets out the four carriers and their costs.

## 2. Candidate table

Dependency column states direct dependencies only, from a primary source, with the
version pinned. The transitive count is measured by the sibling `cargo tree` task.

| Candidate | What is signed | Key and signature formats | Key identified by | Offline | Linux / macOS / Windows | Direct deps (pinned source) |
| --- | --- | --- | --- | --- | --- | --- |
| Detached Ed25519, raw RFC 8032 | The whole artifact byte string (or Ed25519ph over a context) | Public key 32 octets, signature 64 octets (`rfc8032` 5.1.6, 5.1.7) | Nothing built in; the caller must bind a key, so the project would have to define a key id | Yes | All three: pure Rust `ed25519-dalek` 3.0.0 | `curve25519-dalek` 5.0.0, `ed25519` 3, `signature` 3 (optional), `sha2` 0.11, `subtle` 2.3, per the crate's own `Cargo.toml` (`https://docs.rs/crate/ed25519-dalek/3.0.0/source/Cargo.toml.orig`) |
| Detached Ed25519, minisign format | The whole file (legacy `Ed`) or the BLAKE2b-512 prehash of it (`ED`) | 4-line text file: untrusted comment, `base64(algorithm \|\| key_id \|\| signature)`, trusted comment, `base64(global_signature)`; algorithm `Ed` legacy or `ED` hashed; key id 8 random bytes | The 8-byte key id, which matches the public key | Yes | All three: pure Rust `minisign` / `minisign-verify` | Verify-only: `minisign-verify` 0.3.0, zero dependencies. Signing: `minisign` 0.10.0, four direct dependencies as above |
| Detached Ed25519, signify format | The whole message file | Same file shape as minisign minus the trusted comment: line 1 free comment, line 2 `base64(algorithm \|\| key_number \|\| signature)`; 2-byte algorithm `Ed`, 8-byte key number, 64-byte signature | 8-byte key number; `signify -t keytype` additionally constrains the key to `/etc/signify/*-keytype.pub` | Yes | All three (the format is portable; the `signify(1)` tool is OpenBSD's) | No version-pinned Rust crate found in this pass |
| `ring` as the Ed25519 implementation | Same as detached Ed25519 | `signature::Ed25519` with 32-byte public key and 64-byte signature | Caller-supplied | Yes | Builds on all three, but needs a working C toolchain and per-target assembly | `cfg-if` 1.0, `getrandom` 0.4.3, `untrusted` 0.9, `libc` / `windows-link` per target, `cc` build dependency; ships `crypto/*.c` and perlasm (`https://raw.githubusercontent.com/briansmith/ring/main/Cargo.toml`) |
| Signature entry inside the ZIP, JAR-style | Per-entry content digests in `META-INF/MANIFEST.MF`, plus the `.SF` digest of the manifest, plus the PKCS#7 signature over the `.SF` | JAR: `META-INF/*.SF`, `META-INF/*.(RSA\|DSA\|EC)` holding a PKCS#7 CMS `SignedData` (`https://docs.oracle.com/en/java/javase/17/docs/specs/jar/jar.html`) | X.509 signer certificate inside the CMS blob | Yes, once the CMS blob is parsed | Format is portable; a Rust implementation must supply CMS parsing or replace CMS with a raw Ed25519 signature | No version-pinned crate measured here |
| Signature block inside the ZIP, APK v2/v3-style | A two-level digest over the ZIP content sections plus the central directory and the end-of-central-directory record; the digest is inside a signed `signed data` block | Signed block before the central directory: ID-value pairs, X.509 certificate chain, signature-algorithm IDs, and the signature over `signed data` (`https://source.android.com/docs/security/features/apksigning/v2`) | X.509 certificate chain; v3 adds a proof-of-rotation lineage | Yes | Portable in principle; the format is Android's and the host would implement it | No version-pinned crate found |
| Signature in the ZIP comment, or in the central-directory digital-signature record | Whatever the implementer defines, because the specification defines only the framing | APPNOTE defines `Digital signature: header signature 4 bytes (0x05054b50), size of data 2 bytes, signature data (variable size)` and `.ZIP file comment length 2 bytes` / `.ZIP file comment (variable size)`, with no content format for either (`https://pkware.cachefly.net/webdocs/APPNOTE/APPNOTE-4.5.0.txt`) | Implementer-defined | Yes, if the scheme is self-contained | Portable | Reuses the existing ZIP reader |
| sigstore / cosign keyless | The artifact bytes (message signature), or a DSSE envelope; the digest is recorded in a transparency log entry | Sigstore bundle: `verificationMaterial` (X.509 leaf certificate or public-key hint, transparency-log entries with signed entry timestamps, or an RFC 3161 timestamp) plus the signature content (`https://docs.sigstore.dev/about/bundle/`) | The OIDC identity in the Fulcio certificate SAN plus the OIDC issuer, both verified against host-supplied expected values (`https://github.com/sigstore/cosign/blob/main/doc/cosign_verify-blob.md`) | No, not as shipped: see section 3.7 | `cosign` is a Go binary available on all three; binding the Rust `sigstore` crate 0.14.0 adds `aws-lc-rs`, `rustls-webpki`, `rustls-pki-types`, `pkcs8`, `pem`, `scrypt`, `chrono`, `serde` (`https://crates.io/api/v1/crates/sigstore/0.14.0/dependencies`) | See cell at left |
| Platform-native, Authenticode | The hash of a PE image, with some fields excluded, or a catalog file over other file types | PKCS#7 `SignedData` embedded in the PE certificate table, or a separate catalog file | X.509 chain to a root in the Windows trusted root certificate store | Verification is local, but chain and revocation checking is trust-store and CRL/OCSP dependent | Windows only (`WinVerifyTrust`); no equivalent API on Linux or macOS | `windows-sys` or the Win32 API |
| Platform-native, codesign | The Mach-O binary, and for a bundle the sealed resources; nested code is verified only with `--deep` | Apple CMS-based code signature embedded in the Mach-O or a detached signature file | Apple-issued Developer ID certificate; the designated requirement | Local verification is possible; notarization requires Apple infrastructure, and `--check-notarization` forces an online check | macOS only (`SecStaticCodeCheckValidity`) | None for the runtime (the host would call the OS) |
| TUF-style metadata | A signed metadata tree: `root.json` names the keys for all top-level roles, `targets` names target files with hashes, `snapshot` and `timestamp` provide freshness | Canonical JSON with signatures; roles have key lists and thresholds | Role keys listed in `root.json` | No, not in the specified workflow: the client downloads `N+1.root.json`, timestamp, snapshot, and targets, and checks expiration against a fixed update start time | Portable, but the Rust client is heavy | `tough` 0.24.0 direct deps include `aws-lc-rs` 1, `rustls` 0.23, `reqwest` 0.13 (optional), `tokio` 1, `futures` 0.3, `jiff` 0.2, `globset`, `walkdir`, `url`, `tempfile` (`https://docs.rs/crate/tough/latest`) |

## 3. Candidate notes

### 3.1 Detached Ed25519, raw

**What is signed:** the message bytes, exactly as Ed25519 defines them.
**Formats:** 32-octet public key, 64-octet signature; RFC 8032 verification rejects
a signature whose `S` half is out of range and any failed point decoding
(`https://www.rfc-editor.org/rfc/rfc8032.txt`, section 5.1.7: "Decode the first
half as a point R, and the second half as an integer S, in the range 0 <= s < L.
Decode the public key A as point A'. If any of the decodings fail (including S
being out of range), the signature is invalid.").
**Key identification:** none. The verifier must be told which key, or try all
trusted keys. This is the reason to prefer minisign's key id over raw RFC 8032 in a
policy that may hold several keys.
**Implementation warning, grounded:** `ed25519-dalek` 3.0.0 has a
`legacy_compatibility` feature described in its manifest as "Turns off stricter
checking for scalar malleability in signatures"
(`https://docs.rs/crate/ed25519-dalek/3.0.0/source/Cargo.toml.orig`). Default
features are `fast` and `zeroize`. Do not enable `legacy_compatibility`.
**Detection:** all five cases, because the whole byte string is covered.

### 3.2 Detached Ed25519, minisign format

**What is signed:** in legacy `Ed` mode, `ed25519(<file data>)`; in hashed `ED`
mode, `ed25519(Blake2b-512(<file data>))`. The signature file also carries a
`global_signature` computed as `ed25519(<signature> || <trusted_comment>)`, so the
trusted comment is authenticated and the untrusted comment is not
(`https://jedisct1.github.io/minisign/`). Researcher interpretation: the hashed
mode therefore signs the 64-byte BLAKE2b digest as the Ed25519 message, which is
plain Ed25519 with a digest as input, not RFC 8032's Ed25519ph construction. The
specification line quoted above is the direct evidence; the interpretation is mine.

**Key and signature formats:** signature file is four lines (untrusted comment,
`base64(signature_algorithm || key_id || signature)`, trusted comment,
`base64(global_signature)`); public key file is two lines,
`base64(signature_algorithm || key_id || public_key)` with algorithm `Ed` and an
8-byte key id. The secret key file decodes to `algorithm || KDF algorithm || cksum
algorithm || kdf_salt || kdf_opslimit || kdf_memlimit || keynum_sk`, where the KDF
is `Sc` (scrypt), the checksum is `B2` (BLAKE2b-256), the salt is 32 random bytes,
and `keynum_sk = kdf_output XOR (key_id || secret_key || public_key || checksum)`
(104 bytes). So the private key is encrypted at rest with a password, with a
checksum that detects a wrong password, and `-W` is needed to store it unencrypted.
Same source.

**Key identification:** the 8-byte key id.
**Offline:** yes. **Platforms:** the format is byte-oriented and the Rust crates are
pure Rust, so all three targets are fine.
**Interop:** "Minisign creates signatures that can be verified by `signify`;
however, signatures created by `signify` cannot be verified with Minisign because
Minisign expects a trusted comment section to be present"
(`https://jedisct1.github.io/minisign/`). Adopting minisign therefore also accepts
signify-produced signatures only if the verifier tolerates a missing trusted
comment, which `minisign-verify` 0.3.0 supports through its `allow_legacy`-style
arguments in the crate README example (`https://docs.rs/crate/minisign-verify/0.3.0`).
Researcher note: the exact parameter semantics are worth a read of the crate API
before the format freezes.
**Streaming:** `minisign-verify` documents streaming verification that "only works
with pre-hashed signatures" (`https://docs.rs/crate/minisign-verify/0.3.0`), which
matters for a bundle large enough that hashing it in one buffer is undesirable.
**Detection:** all five cases.

### 3.3 Detached Ed25519, signify format

**What is signed:** the whole message file, or a checksum list in `-C` mode, or, in
`-z` mode, a gzip archive with the signing data inside the gzip header
(`https://man.openbsd.org/signify.1`).
**Formats:** "The key and signature files created by `signify` have the same
format. The first line of the file is a free form text comment that may be edited,
so long as it does not exceed a single line. The second line of the file is the
actual key or signature base64 encoded." Same source.
**Key identification:** an 8-byte key number, plus an optional key-type constraint
(`-t keytype`: "make sure the actual key matches `/etc/signify/*-keytype.pub`").
**Gap relative to minisign:** no trusted comment, so no signed place for the bundle
name or version and no downgrade protection. The minisign page lists trusted
comments as the mechanism "to prevent downgrade attacks".
**Detection:** all five cases for the covered file. For a release set, the `-C`
checksum-list mode is the multi-file analogue: it verifies the signed checksum list
and then each file's checksum, which detects a modified or removed member but only
insofar as the list enumerates files and absence is checked.
**Verdict:** the format is sound but is a strict subset of minisign for this use
case. Prefer minisign and keep signify compatibility as a bonus.

### 3.4 `ring`

Not a scheme, an implementation. Relevant cost evidence: `ring` 0.17 declares
`build-dependencies cc`, a `links` attribute (`ring_core_0_17_16000_`), and an
include list containing `crypto/**/*.c`, perlasm scripts such as
`crypto/curve25519/asm/x25519-asm-arm.S`, and `pregenerated/*`, plus a
`third_party/fiat` tree (`https://raw.githubusercontent.com/briansmith/ring/main/Cargo.toml`).
It builds C and per-target assembly, so it is the heaviest of the Ed25519 options
and it buys nothing for Ed25519 verification that a pure Rust verifier does not.
Its direct Rust dependencies are small (`cfg-if`, `getrandom`, `untrusted`, plus
per-target `libc` or `windows-link`), so the cost is in the build, not the crate
graph.

### 3.5 A signature entry inside the ZIP

**JAR-style (digest manifest plus signature entry).** What is signed: the digest of
each entry's uncompressed content, recorded in `META-INF/MANIFEST.MF`; the digest
of the manifest, recorded in `META-INF/<signer>.SF`; and the `.SF` itself, signed
by the CMS blob in `META-INF/<signer>.(RSA|DSA|EC)`. The JAR specification
enumerates the signature-related files and says that the signature block "stores
the digital signature of the corresponding signature file in a PKCS #7 structure"
(`https://docs.oracle.com/en/java/javase/17/docs/specs/jar/jar.html`). Android's
description of the same scheme states the protection chain explicitly: signer
blob, then `.SF`, then `MANIFEST.MF`, then each entry's content
(`https://source.android.com/docs/security/features/apksigning/v2`).

**What it misses, with sources:**
- Added entries: Android has to add a rule for this, which standard JAR semantics do
  not state: "APK verification fails if the APK contains JAR entries which are not
  listed in the `MANIFEST.MF` and are not part of JAR signature." Same source. So
  a JAR-style scheme in Rust would have to invent and enforce the same rule.
- Removed entries: nothing in the digest chain notices that a listed entry is
  simply absent; the verifier's rule would have to be set equality between the
  manifest's entry list and the archive's entry list. This rule is not in the JAR
  specification as quoted above.
- ZIP metadata outside the digest set: entry names are the manifest's keys, but
  local and central-directory headers, extra fields, comments, ordering, and the
  end-of-central-directory record are not covered by any entry digest. This is the
  class of bytes where ZIP-parsing discrepancies live.

**APK v2/v3-style block.** This is the strongest embedded design and it is
whole-file: "APK signature scheme v2 protects the integrity of sections 1, 3, 4,
and the `signed data` blocks" (the ZIP entry contents, the central directory, and
the end-of-central-directory record), using a two-level digest and, for the EOCD,
treating the central-directory offset field as containing the signing block's
offset so the digest is independent of the block's own size. Its verification
rules are exactly the anti-tampering rules this project needs: two size fields must
match, "ZIP Central Directory is immediately followed by ZIP End of Central
Directory record", and "ZIP End of Central Directory is not followed by more data"
(`https://source.android.com/docs/security/features/apksigning/v2`). Rollback and
signature-stripping mitigations are explicit: the signed list of algorithm IDs must
match the list of signatures, and a v1-signed APK must declare the schemes it
carries in `X-Android-APK-Signed`.
**Cost:** the host implements the block format, the two-level digest, the offset
normalization, and the entry-order and set rules. The APK block IDs are Android's,
so adopting the format means adopting APK semantics or inventing parallel IDs, and
the "no data after the EOCD" rule contradicts nothing in this project but does
constrain the packaging tool.
**Detection if implemented exactly as specified:** all five cases.

### 3.6 A signature in the ZIP comment, or in the `0x05054b50` record

**What the specification gives you:** framing only. APPNOTE 4.5.0 lists, in the
central-directory structure, `[digital signature]` with the layout
`header signature 4 bytes (0x05054b50), size of data 2 bytes, signature data
(variable size)`, and, in the end-of-central-directory record,
`.ZIP file comment length 2 bytes` followed by `.ZIP file comment (variable size)`,
explained only as "The length of the comment for this .ZIP file"
(`https://pkware.cachefly.net/webdocs/APPNOTE/APPNOTE-4.5.0.txt`). Researcher
inference: the specification defines where a signature can live and how large it
may be (the comment length is a 16-bit field, so 65535 bytes at most), but it
defines no signing input, no algorithm identifier, and no key format. There is
therefore no interoperable scheme to adopt, only a container. APK's designers made
the same judgement and introduced a new block instead of using the standard record.
**Coverage problem:** the comment is inside the container being protected, so the
signing input has to exclude it, and the verifier has to reconstruct exactly that
exclusion. Tools also rewrite or drop comments, which changes the file and
invalidates the signature for reasons that have nothing to do with tampering.
**Reader tolerance, for context:** the `zip` crate 9.0.3 exposes
`ZipArchive::comment()` and documents an offset accessor whose value "will be the
size of that prepended data" when "the zip has arbitrary data prepended to it"
(`https://docs.rs/zip/latest/zip/read/struct.ZipArchive.html`). Researcher
inference: readers tolerate bytes around the archive, so the signature check, not
the reader, has to be the thing that decides whether the file is exactly the byte
string that was signed.
**Detection:** depends entirely on the scheme the implementer defines; the
container itself detects nothing.

### 3.7 sigstore and cosign keyless

**What is signed:** the artifact bytes, with the verification material carried
alongside. "A Sigstore bundle is everything required to verify a signature on an
artifact", satisfied by verification material (a Fulcio leaf certificate or a
public-key hint, plus transparency-log entries and timestamps) and the signature
content; "bundles must include at least one transparency log's signed entry
timestamp or an RFC3161 timestamp to provide proof that signing occurred during the
certificates validity window" (`https://docs.sigstore.dev/about/bundle/`).
**Key identification:** the OIDC identity, not a key. A Fulcio issued certificate
"MUST specify exactly one Subject Alternative Name, as a critical extension. It
MUST be populated by either an email or a URI", MUST carry the OIDC issuer OID, and
SHOULD use an ephemeral key; "Clients MUST NOT trust certificates that do not
present either a proof of inclusion or a Signed Certificate Timestamp (SCT)"
(`https://raw.githubusercontent.com/sigstore/fulcio/main/docs/certificate-specification.md`).
**Verification requires policy input:** "Either `--certificate-identity` or
`--certificate-identity-regexp` must be set for keyless flows" and the same for the
OIDC issuer (`https://github.com/sigstore/cosign/blob/main/doc/cosign_verify-blob.md`).
**Offline: no, not as shipped.** The cosign maintainer states in the issue that
raised this: "`--offline` is not well documented and doesn't do airgapped
verification. This is to gate whether the client makes requests to the transparency
log when no proof is provided." And: "If you want to have verification be entirely
offline, you'll need to download the trust root file and provide that via
`--trusted-root`... but note that it will eventually become stale and so you'll
need some mechanism to periodically retrieve the latest file." Asked how fast it
goes stale, the answer was "The transparency log key is rotated every year"
(`https://github.com/sigstore/cosign/issues/4454`). The issue includes a failing
`cosign verify-blob --offline` whose error is a DNS failure fetching
`https://tuf-repo-cdn.sigstore.dev/13.root.json`. Signing keyless additionally
requires an OIDC identity provider and a Fulcio round trip, per the same source and
the bundle documentation.
**Rust cost:** the `sigstore` crate 0.14.0 depends directly on `aws-lc-rs`,
`rustls-webpki`, `rustls-pki-types`, `pkcs8`, `pem`, `scrypt`, `chrono`,
`const-oid`, `digest`, `crypto_secretbox`, `base64`, `serde`, `serde_json`, and
`sha2` (`https://crates.io/api/v1/crates/sigstore/0.14.0/dependencies`). That is a
TLS certificate-validation stack and a C crypto library, which is the specific cost
the repository already refused once for `jsonschema` (see the comment in
`crates/plugin-runtime/Cargo.toml` and `Cargo.toml`).
**Detection:** all five cases against the artifact, if the trust root is current
and the identity policy is supplied. The residual risks are freshness and the
external dependency, not the coverage model.

### 3.8 Platform-native signatures

**Authenticode.** The signature lives in a PE certificate table or in a catalog
file: "In Windows, a file's digital signature may be stored in either a catalog
signature, an embedded signature, or both" and "the SIP may exclude some sections
of the file from the hash calculation" (`https://learn.microsoft.com/en-us/windows/win32/secbp/understanding-pe-signatures`).
The same page states the consequence plainly: because the provider "does not hash
all of the bytes of the file", "it is possible to embed additional content into a
signed PE file without breaking its signature", which was exploited from 2013 and
forced the `EnableCertPaddingCheck` override for CVE-2013-3900. Trust is a chain to
the platform store: the documented policy checks that "The certificate used to sign
the file chains up to a root certificate located in the trusted root certificate
store" (`https://learn.microsoft.com/en-us/windows/win32/seccrypto/example-c-program--verifying-the-signature-of-a-pe-file`),
and revocation is a separate chain operation
(`https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certverifyrevocation`).
For a file type with no installed SIP, "the entire content of the file may be
hashed... and stored in a Catalog file"; and "When verifying a Catalog-signed file,
the SIP is loaded only for signing and validating the PE and CAB file types". A
`.metis` ZIP has no SIP, so the only whole-file option is a catalog file, which is
machine-local registered state rather than something that travels with the bundle.
Grounding is from the same Microsoft page. `WinVerifyTrust` is a Windows API, so
this is unavailable on Linux and macOS.
**codesign.** macOS code signing is mandatory for distribution outside the App
Store: "all apps distributed outside the App Store need to be signed by the
developer using an Apple-issued Developer ID certificate (combined with a private
key) and notarized by Apple to run under the default Gatekeeper settings", and the
notarization output is a ticket "which is stored on Apple servers and can be
optionally stapled to the app" (`https://support.apple.com/guide/security/app-code-signing-process-sec3ad8e6e53/1/web/1`).
Verification of nested content is shallow by default: with `--deep`, "any nested
code content will be recursively verified... By default, verification of nested
content is limited to a shallow investigation that may not detect changes to the
nested code", and `--check-notarization` forces an online check
(`https://keith.github.io/xcode-man-pages/codesign.1.html`). The same page warns
that "Verification/validation do not check the signature against OS policy. A
valid/verified signature may still fail to meet Gatekeeper, Entitlements or other
contextual policy requirements." Researcher inference: the codesign path is macOS
only, it signs the Mach-O library and the sealed bundle contents rather than
`manifest.toml` as an archive-level claim, and the notarization leg adds an Apple
account dependency.
**Detection summary for both:** a swapped library is detected if that library is
itself signed and the host checks it; a modified manifest is not covered; added and
removed archive entries are not covered; truncation is not covered. They are
binary-signing systems, not archive-integrity systems.

### 3.9 TUF-style metadata

**Shape and sources.** "There are four fundamental top-level roles in the
framework: Root role, Targets role, Snapshot role, Timestamp role", and "All roles
can use one or more keys and require a threshold of signatures of the role's keys
in order to trust a given metadata file." The root role "delegates trust to
specific keys trusted for all other top-level roles used in the system. The
client-side of the framework MUST ship with trusted root keys for each configured
repository." (`https://theupdateframework.github.io/specification/latest/`)
**Rotation and revocation, sourced.** "Revocation and replacement of top-level role
keys, including for the root role, is done by changing the keys listed for the
roles in this file [root.json]", and "When replacing root keys, an application will
sign the new root.json file with both the new and old root keys. Any time such a
change is required, the root.json file is versioned and accessible by version
number". If less than a threshold of root keys is compromised, "the repository
should revoke trust on the compromised keys. This can be accomplished with a normal
rotation of root keys". "All keys, except those for the timestamp and mirrors
roles, should be stored securely offline." Delegations can be revoked "at any
time: the delegating role needs only to sign new metadata that no longer contains
that delegation". Same source. Also stated as a design goal: "The root of trust
must not rely on external PKI."
**Offline mismatch, sourced.** The client workflow is a download loop: the client
records a fixed update start time, must "Try downloading version N+1 of the root
metadata file", must "Check for a freeze attack. The expiration timestamp in the
trusted root metadata file MUST be higher than the fixed update start time", and
must delete timestamp and snapshot metadata when those keys rotate. Applying TUF to
a locally supplied bundle therefore either imports a freshness cycle that a local
file does not need, or reduces to a signed list of target hashes, which is the
digest-manifest scheme with extra machinery.
**Rust cost, sourced:** `tough` 0.24.0 depends directly on `aws-lc-rs` 1,
`rustls` 0.23, `reqwest` 0.13 (optional), `tokio` 1, `futures` 0.3, `jiff` 0.2,
`serde`/`serde_json`, `snafu`, `tempfile`, `globset`, `walkdir`, `url`, `hex`,
`pem`, `percent-encoding`, `typed-path` (`https://docs.rs/crate/tough/latest`).
**Where TUF's value survives:** the root-signs-plugin-keys idea. That part is worth
keeping as an option, without the metadata tree (section 6).

## 4. Failure-mode matrix

Rows are the five required tampering cases. "Yes" means the scheme, implemented as
its primary source defines it, detects the case.

| Scheme | Modified `manifest.toml` | Swapped library | Added entry | Removed entry | Truncated file |
| --- | --- | --- | --- | --- | --- |
| Detached Ed25519 over the whole file (raw, minisign, signify) | Yes | Yes | Yes | Yes | Yes |
| Signature entry, JAR-style digest manifest | Yes, if the signed manifest digests `manifest.toml` | Yes, entry content digest | Only with an explicit "no entry outside the manifest" rule, which Android had to add | Only with an explicit set-equality rule, which the JAR specification does not state | Yes for a truncated member; a truncation that removes trailing entries is a structural refusal the host must implement, not a signature failure |
| Signature block, APK v2/v3-style | Yes | Yes | Yes, whole-file digest plus the "EOCD not followed by more data" rule | Yes | Yes |
| ZIP comment or `0x05054b50` record | Depends on the implementer's signing input; the specification defines no content format | Same | Same | Same | Same |
| sigstore keyless | Yes | Yes | Yes | Yes | Yes, subject to trust-root freshness and identity-policy inputs |
| Authenticode on the library | No, the manifest is not in the PE | Yes, if that library is signed and checked | Only with `EnableCertPaddingCheck` enabled; the hash excludes bytes by design | No | No |
| codesign on the library | No, unless the manifest is inside a sealed bundle | Yes for sealed code; nested changes may be missed without `--deep` | Depends on what is sealed | Depends on what is sealed | No |
| TUF metadata plus target hashes | Yes, if `manifest.toml` is a target | Yes | Yes, via the container's target hash | Yes | Yes, via hash and length |

## 5. The carrier question for an in-band signature

A whole-file signature over a single-file artifact forces a decision about where the
signature bytes live, because the signature cannot be inside the byte range it
covers.

1. **Sidecar** (`bundle.metis` plus `bundle.metis.minisig`). Coverage is
   unambiguous, it is what minisign and signify tooling produce naturally, and the
   host verifies before it opens the archive at all. Cost: two files, and the
   policy must decide what an absent signature means (recommendation: refuse, that
   is `compatibility-policy`'s call).
2. **Trailer** (signature appended after the ZIP bytes; the signed range is the
   prefix). Single file, whole-file coverage, no self-reference. Cost: the file is
   no longer strictly the ZIP the specification describes, because "APK signature
   scheme v2 verification" requires that the end-of-central-directory record "is
   not followed by more data"
   (`https://source.android.com/docs/security/features/apksigning/v2`), and the
   host must define a strict rule that nothing follows the trailer.
3. **ZIP comment** (end-of-central-directory comment). Keeps the file a valid ZIP,
   but the comment is inside the container, so the signing input must be "the file
   with the comment removed and its 2-byte length zeroed", the field is capped at
   65535 bytes, and ZIP tools may rewrite it.
4. **Signature entry** inside the archive. Requires either the JAR or the APK model
   and their extra rules, and it means the host parses untrusted archive structure
   before it has authenticated anything.

Researcher recommendation: sidecar for v1, with the trailer as the fallback if
single-file transport is a hard requirement. Reject 3 and 4 for v1: they add
coverage rules without adding coverage.

## 6. Trust-chain and key-custody options

### 6.1 Flat trusted key set (recommended)

Shape: the daemon supplies a set of entries `(key_id, 32-byte public key)`. The
bundle carries or is accompanied by a key id, and the host refuses when the id is
absent from the set or the signature does not verify. Prior art, both third-party
and therefore corroborating rather than authoritative: `uni-db` builds a `TrustRoot`
with `allow_with_key(key_id, public_key)` binding "a key id to its 32-byte Ed25519
public key", keeps the verifier "fail-closed", and holds the policy as a host-level
runtime object rather than in serializable configuration
(`https://docs.rs/uni-db/latest/src/uni_db/api/plugin_trust.rs.html`). `polyplug`
pins trusted keys as "the raw 32-byte verifying-key encoding", keeps only public
keys pinned ("Only public verifying keys are pinned; the private signing key stays
offline with the author"), and separates integrity from authenticity
(`https://github.com/polyplug/polyplug/blob/main/docs/TRUST_MODEL.md`).

- **Distribution:** the public key set is host input, matching the constraint that
  the runtime never reads the daemon's configuration. A plugin author obtains a
  trusted key by getting the operator to add the public key and key id. There is no
  key server in this shape.
- **Rotation:** add the new key id to the policy, sign new bundles with the new key,
  and remove the old key id after a published overlap window. Old bundles do not
  need re-signing while the old key id remains.
- **Withdrawal:** remove the key id. This is immediate, local, and offline, and it
  requires no clock, which is a real advantage over any scheme with expiry. The cost
  is that withdrawal is a policy redistribution event.
- **Custody:** the private key is the minisign secret key file, scrypt-encrypted
  with a BLAKE2b-256 checksum over `algorithm || key_id || secret_key || public_key`
  (`https://jedisct1.github.io/minisign/`). The build machine needs the private key
  and its passphrase only in the signing step; the host needs only the 32-byte
  public keys. The Rust `minisign` crate supports generating an encrypted key pair
  from a password and signing from a loaded secret key box
  (`https://docs.rs/crate/minisign/0.10.0`).

### 6.2 A root key that signs plugin keys (deferred)

Shape: the bundle carries a plugin key plus a root-signed record binding that key
to the plugin identity, so a new author can onboard without the host editing its
policy. Sources for the two established designs:

- TUF's delegation model: the root role "delegates trust to specific keys trusted
  for all other top-level roles"; keys for a role are replaced by signing a new
  `root.json` listing updated keys, signed with both the new and the old root keys
  when root keys themselves change; expired metadata is checked against a fixed
  update start time, and all keys except timestamp and mirrors should be offline
  (`https://theupdateframework.github.io/specification/latest/`).
- APK v3's compact version: a proof-of-rotation attribute inside the signed block,
  "a singly-linked list, with each node containing a signing certificate used to
  sign previous versions of the app... built by having the cert in each node sign
  the next in the list", and "Because the whole proof-of-rotation attribute resides
  in the signed data section of the v3 `signer` field, it is protected by the key
  used to sign the containing apk" (`https://source.android.com/docs/security/features/apksigning/v3`).

Costs and open points: a delegation record format, whether it expires (which
imports a clock and a refresh mechanism, the thing TUF needs a timestamp role for),
and how a compromised plugin key is withdrawn (a negative list in the host policy,
or expiry).
Researcher recommendation: do not build this in v1. Adopt it when third-party
authors must onboard without an operator policy edit, and then prefer the APK v3
lineage shape over the full four-role TUF tree, because the lineage is one signed
record inside the bundle and needs no server.

### 6.3 Platform trust stores (rejected for v1)

Authenticode trust is a chain to a root in the Windows trusted root certificate
store (`https://learn.microsoft.com/en-us/windows/win32/seccrypto/example-c-program--verifying-the-signature-of-a-pe-file`),
with revocation as a separate chain operation
(`https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certverifyrevocation`).
macOS requires an Apple-issued Developer ID certificate and notarization for
distribution (`https://support.apple.com/guide/security/app-code-signing-process-sec3ad8e6e53/1/web/1`).
Linux has no equivalent store for arbitrary user artifacts. The shape therefore
fails the three-target requirement on its own terms, and it makes CA policy and
revocation availability part of plugin loading.

## 7. Rejected candidates and why each fails

1. **sigstore / cosign keyless.** Fails the offline requirement as documented by
   the maintainer: `--offline` "doesn't do airgapped verification", a fully offline
   setup needs a `trusted_root.json` from a TUF repository, and that file "will
   eventually become stale... you'll need some mechanism to periodically retrieve
   the latest file" because "the transparency log key is rotated every year"
   (`https://github.com/sigstore/cosign/issues/4454`). Signing keyless also needs an
   OIDC identity provider and a Fulcio round trip, and the Rust binding pulls a TLS
   validation stack and `aws-lc-rs`
   (`https://crates.io/api/v1/crates/sigstore/0.14.0/dependencies`).
2. **Platform-native signatures.** Fail on availability: `WinVerifyTrust` is
   Windows only and codesign is macOS only, so one bundle cannot be verified on all
   three targets. They also fail on payload: Authenticode's hash "does not hash all
   of the bytes of the file", so embedding content in a signed PE does not break the
   signature without `EnableCertPaddingCheck`
   (`https://learn.microsoft.com/en-us/windows/win32/secbp/understanding-pe-signatures`),
   and a ZIP has no SIP, leaving only a machine-local catalog file. codesign does
   not cover the manifest, and its default nested verification "may not detect
   changes to the nested code"
   (`https://keith.github.io/xcode-man-pages/codesign.1.html`).
3. **TUF-style metadata as the v1 shape.** Not a security failure but a mechanism
   mismatch: the specified client workflow is a download-and-refresh cycle with a
   fixed update start time, expiration checks, and freeze-attack detection, none of
   which a locally supplied file needs, while the parts that do help (root key signs
   plugin keys, target hashes) are available more cheaply. It also brings the
   heaviest Rust client measured here (`aws-lc-rs`, `rustls`, `tokio`, optional
   `reqwest`): `https://docs.rs/crate/tough/latest`.
4. **A signature entry inside the ZIP, JAR-style.** Fails on completeness unless
   the project re-derives Android's rules: added-entry detection is an Android
   addition ("APK verification fails if the APK contains JAR entries which are not
   listed in the `MANIFEST.MF`..."), and removed-entry detection is not stated at
   all. It also leaves ZIP headers, extra fields, comments, and ordering outside the
   signed set. And it requires parsing untrusted archive structure before anything
   is authenticated.
5. **A signature entry inside the ZIP, APK-style.** This one works, and its
   verification rules are the right ones, but it is the most code for the same
   coverage a whole-file detached signature gets for free: two-level digesting,
   normalization of the central-directory offset field, ID-value block framing, and
   the "no data after the EOCD" packaging constraint
   (`https://source.android.com/docs/security/features/apksigning/v2`). Rejected on
   cost, not on soundness.
6. **A signature in the ZIP comment, or in the `0x05054b50` record.** No
   interoperable scheme exists to adopt: the specification gives framing
   (`header signature 4 bytes (0x05054b50), size of data 2 bytes, signature data`)
   and a 16-bit comment length field, with no signing input or algorithm
   (`https://pkware.cachefly.net/webdocs/APPNOTE/APPNOTE-4.5.0.txt`). The
   implementer would define a bespoke scheme whose coverage rules are as intricate
   as the in-band options above and whose bytes ZIP tools may rewrite.
7. **`ring` as the host verifier.** Not unsound, but it vendors C and per-target
   assembly and adds a `cc` build step and a `links` attribute
   (`https://raw.githubusercontent.com/briansmith/ring/main/Cargo.toml`), for a
   verification task that pure Rust verifiers already do, one of them with zero
   dependencies.
8. **The signify format without a trusted comment.** Usable, but it is a strict
   subset of minisign for this use case: no signed place to bind the bundle name
   and version, which is the documented downgrade protection. Prefer minisign and
   keep the ability to read signify signatures.

## 8. Unresolved questions the operator must settle in `plugin-signing`

1. **Carrier:** sidecar, trailer, ZIP comment, or signed entry (section 5). This
   changes the SDK CLI, the file extension story, and whether a `.metis` file alone
   is a complete artifact.
2. **Trust-chain shape for v1:** flat key set only, or root key that signs plugin
   keys. If the latter, who holds the root key, is the delegation record expiring,
   and how is a plugin key withdrawn.
3. **The exact host input shape for the trust policy:** the runtime receives the
   policy as input, but the type (key id plus key bytes, a parsed structure, a file
   path), the delivery time (once at startup or per load), and the failure mode for
   an unknown key id are unspecified in the map.
4. **What an unsigned or untrusted bundle gets.** Fail closed, warn and load, or
   refuse only under a strict policy. This is `compatibility-policy`'s decision, but
   the runtime must expose the hook it needs.
5. **Rotation and withdrawal procedure:** overlap window, whether old bundles are
   re-signed, and whether the policy may express a revocation time (which imports a
   clock).
6. **Re-targeting:** whether a signed bundle may be re-packaged for another target
   without re-signing. A whole-file signature forbids it. Allowing it requires a
   digest-manifest scheme and therefore re-imports the added and removed entry
   problems in section 3.5.
7. **Trusted-comment content:** whether the bundle name, bundle format version, and
   plugin version go into the trusted comment as minisign's documentation suggests
   for downgrade protection, and in what exact grammar, since the host would parse
   it.
8. **Key id derivation:** minisign's is 8 random bytes from the key pair. If the
   project defines its own format instead, decide whether the id is random or a hash
   prefix, before either format freezes.
9. **Who signs, and where the private key lives:** the SDK CLI at build time, a
   separate release step, or CI with a password-protected key from a secret store.
10. **Whether the host records the verifying key id per load** in the registry or
    its log, for audit and for answering "which key signed the plugin that is
    running".
11. **Whether the ZIP reader's tolerance around the archive is allowed to matter.**
    Researcher recommendation: it must not. The signature must be evaluated over
    exactly the byte string that was signed, and any byte outside that string,
    before or after, must be a refusal rather than something a lenient reader
    forgives.

## 9. Sources

Kept:

- `https://www.rfc-editor.org/rfc/rfc8032.txt` (RFC 8032): Ed25519 key and signature sizes and the verification rule that rejects out-of-range `S`.
- `https://jedisct1.github.io/minisign/`: the minisign signature, public key, and secret key formats, the trusted comment and global signature, the prehash rule, and the signify interoperability statement.
- `https://man.openbsd.org/signify.1`: the signify key and signature file shape, the key-type hint, and the checksum-list and gzip modes.
- `https://docs.rs/crate/minisign-verify/0.3.0` and the crates.io dependency API for `minisign-verify` 0.3.0: zero dependencies and streaming verification of prehashed signatures.
- `https://docs.rs/crate/minisign/0.10.0` and the crates.io dependency API for `minisign` 0.10.0: pure Rust signing path, encrypted key pairs, four direct dependencies.
- `https://docs.rs/crate/ed25519-dalek/3.0.0/source/Cargo.toml.orig`: direct dependencies, default features, and the `legacy_compatibility` malleability switch.
- `https://raw.githubusercontent.com/briansmith/ring/main/Cargo.toml`: the C and assembly include list, the `cc` build dependency, and the `links` attribute.
- `https://pkware.cachefly.net/webdocs/APPNOTE/APPNOTE-4.5.0.txt`: the central-directory digital signature record, the end-of-central-directory comment fields, and the ZIP structural layout.
- `https://docs.oracle.com/en/java/javase/17/docs/specs/jar/jar.html`: the signed JAR protection chain and the list of signature-related files.
- `https://source.android.com/docs/security/features/apksigning/v2`: whole-file coverage model, the two-level digest, the EOCD integrity rules, the added-entry rule, and the rollback and stripping mitigations.
- `https://source.android.com/docs/security/features/apksigning/v3`: the proof-of-rotation lineage and its protection by the containing signature.
- `https://docs.sigstore.dev/about/bundle/`: what a Sigstore bundle contains and the timestamp requirement for verifying an expired certificate.
- `https://raw.githubusercontent.com/sigstore/fulcio/main/docs/certificate-specification.md`: the issued certificate profile, the single SAN, the OIDC issuer OID, and the SCT or inclusion-proof requirement.
- `https://github.com/sigstore/cosign/issues/4454`: the maintainer's statement that `--offline` is not airgapped verification, that a trusted root must be fetched from a TUF repository, and that the transparency log key rotates yearly.
- `https://github.com/sigstore/cosign/blob/main/doc/cosign_verify-blob.md`: the required identity and issuer inputs for keyless verification and the `--trusted-root` option.
- `https://crates.io/api/v1/crates/sigstore/0.14.0/dependencies`: the direct dependency list of the Rust sigstore client.
- `https://learn.microsoft.com/en-us/windows/win32/secbp/understanding-pe-signatures`: catalog versus embedded signatures, the excluded hash ranges, the unsigned-embedding risk and CVE-2013-3900, and the SIP limitation to PE and CAB.
- `https://learn.microsoft.com/en-us/windows/win32/seccrypto/example-c-program--verifying-the-signature-of-a-pe-file`: the trust-store chain check in the documented Authenticode policy.
- `https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certverifyrevocation`: revocation as a separate chain operation.
- `https://support.apple.com/guide/security/app-code-signing-process-sec3ad8e6e53/1/web/1`: the Developer ID and notarization requirement and the stapled ticket.
- `https://keith.github.io/xcode-man-pages/codesign.1.html`: `--deep` and the shallow default nested verification, the online notarization check, and the "verification does not check OS policy" warning.
- `https://theupdateframework.github.io/specification/latest/`: the four roles, thresholds, root key storage and rotation, delegation revocation, the freeze and rollback goals, and the client update workflow with expiry checks.
- `https://docs.rs/crate/tough/latest`: the direct dependency list of the Rust TUF client.
- `https://docs.rs/zip/latest/zip/read/struct.ZipArchive.html` and `https://docs.rs/zip/latest/zip/read/struct.ZipFile.html`: the archive comment accessor, the prepended-data offset accessor, and the per-file CRC accessor.
- `https://raw.githubusercontent.com/zip-rs/zip2/master/CHANGELOG.md`: the 8.3.0 entry "add crc32 ignore option", which is evidence that a CRC-32 check runs by default.
- `docs/tasks/metis-plugins/map.md`, `docs/tasks/metis-plugins/tasks/plugin-signing-research/task.md`, `docs/plugin-contract.md`, `crates/plugin-runtime/Cargo.toml`, `Cargo.toml`, `docs/testing.md`: local constraints, the extraction decisions already taken, the dependency-budget comment about `jsonschema`, and the gate commands.
- Prior art, corroborating only: `https://github.com/polyplug/polyplug/blob/main/docs/TRUST_MODEL.md` and `https://docs.rs/uni-db/latest/src/uni_db/api/plugin_trust.rs.html` for the Ed25519 plus flat key set plus host-level policy shape.

Rejected or deprioritized:

- Apple Developer documentation pages (`developer.apple.com/documentation/...`): JavaScript-rendered, so the content could not be read; replaced by the static Apple Support guide and the codesign manual page.
- The Authenticode PE signature format PDF hosted at `symbolcrash.com` and `yossarian.net`: secondary copies of a Microsoft document; the live Microsoft Learn page was used instead.
- `rsign2` crate page: it is a CLI wrapper around the `minisign` crate (its dependency list names `minisign`), so the underlying crate was cited instead.
- Various ZIP-format encyclopedia and SEO pages surfaced by search: superseded by APPNOTE 4.5.0 itself.
- The `tuf` crate dependency API endpoints: the version requested did not exist, and `tough` already provides a version-pinned, primary dependency list.

## 10. Confidence and limits

- Confidence in the tamper-case analysis of the whole-file Ed25519 candidates is
  **high**: RFC 8032 and the minisign specification both define the signed input as
  the file bytes, so all five cases are covered by construction.
- Confidence in the JAR-style and ZIP-comment findings is **high** for what the
  specifications state and **medium** for the "what a Rust implementation would have
  to add" part, which is researcher inference from the quoted rules.
- Confidence in the sigstore offline finding is **high**: it rests on a maintainer
  statement and a reproduced failing command in the project's own issue tracker.
- Confidence in the platform-native findings is **high** for Authenticode's excluded
  hash ranges and the Windows-only API surface, and **medium** for the claim that a
  `.metis` bundle cannot carry an embedded Authenticode signature, which is
  inference from "the SIP is loaded only for signing and validating the PE and CAB
  file types" plus the catalog fallback.
- No version-pinned Rust crate was found for the signify format or for the APK
  signature block, so those rows of the table carry no dependency figures rather
  than guesses. Transitive counts throughout are measured by the sibling
  `cargo tree` task, not here.

## 11. Measured dependency cost

Measured 2026-10-14 with `cargo tree --edges normal` on a scratch workspace at
`.work/signing-research/deps/`, each candidate in its own probe crate. Full
per-candidate output and the two complete package lists are in
`.work/signing-research/deps.md`.

| Candidate | Pinned version | Feature set | Distinct packages (incl. probe) | Third-party packages | Direct deps of the crate |
| --- | --- | --- | --- | --- | --- |
| `ed25519-dalek` (signing) | 3.0.0 | `rand_core`, `signature` (+ defaults `fast`, `zeroize`) | 21 | 20 | curve25519-dalek, ed25519, rand_core, sha2, signature, subtle, zeroize |
| `ed25519-dalek` (verify only) | 3.0.0 | `default-features = false` | 19 | 18 | curve25519-dalek, ed25519, sha2, subtle |
| `ring` | 0.17.14 | defaults (`alloc`, `dev_urandom_fallback`) | 6 | 5 | cfg-if, getrandom, untrusted |
| `minisign` | 0.10.0 | defaults (the crate declares no features) | 22 | 21 | ct-codecs, getrandom, rpassword, scrypt |
| `ed25519-compact` | 2.6.0 | defaults (`random`, `std`, `x25519`, `pem`) | 6 | 5 | ct-codecs, getrandom |

Largest tree: `minisign`, 21 third-party packages.
Smallest tree: `ring` and `ed25519-compact` tie, 5 third-party packages each.


Caveats carried from the measurement:

- `--edges normal` hides the `cc` build dependency `ring` declares, and `ring`
  compiles C and per-target assembly through a `links` attribute, so its 6-package
  normal tree understates its real build cost. The researcher note already rejects
  `ring` on those grounds.
- `minisign-verify` 0.3.0, the verify-only crate the host would use, reports an
  empty dependency list from the crates.io dependency API, checked 2026-10-14, so
  the host-side verifier adds no packages at all. The 22-package `minisign` crate
  is the signing side, which belongs to the SDK CLI and the build machine rather
  than to `plugin-runtime`.
- `minisign-verify` cannot sign and `minisign` cannot verify, so a scheme choice
  names two crates, one per side.

