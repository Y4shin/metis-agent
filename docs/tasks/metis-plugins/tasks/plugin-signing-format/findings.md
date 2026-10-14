---
type: findings
title: Which format carries the plugin signing chain and the payload signature
status: stable
---

# Findings: which format carries the plugin signing chain and the payload signature

Produced 2026-10-14 for the `plugin-signing-format` research task. One
researcher child wrote the format survey and one worker child measured the
dependency costs at `.work/signing-format/deps.md`; the parent merged them
into this file, which is the only writer of it. The recommendation below is
decision input for `plugin-signing`, which owns the decision.

## 1. The question, and what is already settled

`plugin-signing` already decided (S1 to S10 in
`docs/tasks/metis-plugins/tasks/plugin-signing/task.md`):

- S1: one file, fixed-size prefix header, then the chain, then the signature,
  then the ZIP bytes. The signed range is the header, the chain and the ZIP
  bytes; the signature is excluded by the offset and length the header declares.
- S2: the daemon holds a set of root keys plus a revocation list; a bundle
  carries a chain from its signing key to a root; no revoked key may appear.
- S6: a failed or absent signature is fatal.
- S7: the key id is derived from the public key, not random.
- S8: a link is a parent signature over the child's public key, an arbitrary
  description of bounded length, and an optional expiry in Unix UTC
  milliseconds; the last signature is the payload signature over header, chain
  and ZIP bytes.
- S10: the operator wants an established format that is as lightweight as
  possible, with either a small Rust implementation or a parser simple enough to
  reimplement safely.

This note decides the chain encoding and the two signature encodings. It does
not reopen the scheme (Ed25519, from
`docs/tasks/metis-plugins/tasks/plugin-signing-research/findings.md`) and it does
not own the header field list, which stays with `plugin-signing`.

Dependency numbers in this note are the measured ones from the sibling
measurement at `.work/signing-format/deps.md`, taken with `cargo tree` on a
scratch workspace.

## 2. Recommendation

**A minimal project format: a fixed-stride, length-prefixed binary link record,
signed by raw Ed25519, with a domain-separated signing input and one canonical
encoding per value.** The payload signature is 64 raw Ed25519 bytes in the
header over the file bytes with that field omitted.

Reason in one sentence: every established candidate either cannot carry the three
S8 fields without a project-defined extension (X.509 extension or DN, CWT custom
claim, OpenPGP notation, minisign trusted comment), or carries them only at a
granularity the operator did not ask for (seconds, not milliseconds), while the
part that makes those formats valuable, a reviewed validator, is precisely the
part they do not deliver for this shape: `x509-cert` is a format library with no
validator (`https://docs.rs/x509-cert/latest/x509_cert/`), `rustls-webpki` cannot
expose the fields the chain must carry
(`https://github.com/rustls/webpki`), `x509-verify` warns "Some of the features
of this crate are in an early, experimental phase. Use at your own risk."
(`https://docs.rs/crate/x509-verify/latest`), and OpenPGP's expiry and
revocation semantics are explicitly outside its specification
(`https://docs.rs/crate/pgp/latest`, section 5.5 below).

The cost also falls the right way. The verification path the whole format needs
is a raw Ed25519 verify over a byte string, which measures 18 third-party
packages in verify-only configuration (`ed25519-dalek` 3.0.0 with
`default-features = false`), and the chain parser is a fixed-offset walk, not a
parser. The cheapest established chain verification measured is `x509-cert` at 13
third-party packages for the format only, with no validator, and `rustls-webpki`
with a working backend at 8 packages that cannot expose the required fields.
Both of those are C-free, but the certificate stack that can actually validate is
not the small one, and the OpenPGP routes are 71 to 146 packages.

## 3. Minimal canonical byte layout, v1

Everything below is written so a second implementer can reproduce it byte for
byte. All integers are unsigned big-endian unless stated. No base64, no JSON, no
CBOR, no ASN.1, no compression in this region.

### 3.1 Link record

A link is `109 + N` bytes, `N = description_len`:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 1 | `link_version`, MUST be `0x01` |
| 1 | 1 | `suite`, MUST be `0x01` (Ed25519, 32-byte keys, SHA-256 key ids) |
| 2 | 2 | `description_len` (`u16`), MUST be `<= 256` |
| 4 | N | `description`, UTF-8, not padded |
| 4 + N | 1 | `expiry_present`, MUST be `0x00` or `0x01` |
| 5 + N | 8 | `expiry_millis`, `i64`, Unix UTC milliseconds |
| 13 + N | 32 | `child_public_key`, raw Ed25519 public key |
| 45 + N | 64 | `signature`, raw Ed25519 signature by the parent key |

The expiry field is always present as 8 bytes so that every later offset is a
constant plus `N`. When `expiry_present` is `0x00`, `expiry_millis` MUST be `0`
and the link has no expiry. When it is `0x01`, `expiry_millis` MUST be `> 0` and
is the instant after which the link is expired. A non-zero `expiry_millis` with
`expiry_present == 0x00` is a refusal, not a silently ignored value.

### 3.2 Link signing input

```
input = "metis-plugin-chain-link-v1" || 0x00 || link_bytes[0 .. 45 + N]
```

i.e. the domain tag, one `0x00` byte, then exactly the link record bytes from
offset 0 up to but not including the signature. The signature is the Ed25519
signature by the parent key over that input.

### 3.3 Chain

The chain is the concatenation of link records, root end first: link 0 is signed
by a key in the daemon's root set, link `i`'s `child_public_key` is the parent
key that signs link `i + 1`, and the last link's `child_public_key` is the leaf
key. The header declares the chain's total byte length; the parser walks records
until that length is consumed exactly. A trailing partial record is a refusal.
The link count is derived, never carried, so a count cannot disagree with the
bytes. Maximum 8 links.

### 3.4 Payload signature

The payload signature is 64 raw Ed25519 bytes at the offset the header declares,
by the leaf key, over

```
"metis-plugin-payload-v1" || 0x00 || (file bytes with the 64 signature bytes omitted)
```

Omission, not zero-filling: the verifier concatenates the bytes before the field
and the bytes after it. The verifier MUST require
`sig_offset + 64 + zip_len == file_len`, where `zip_len` is the header's declared
ZIP length, so no byte of the file escapes the signed range. The header's own
key-id field (S1) MUST be recomputed, not trusted.

### 3.5 Key ids

`key_id(k) = SHA-256(k)[0 .. 16]` over the 32-byte raw public key. Ids are never
carried in v1; the verifier recomputes them, which makes the S7 relabelling
attack impossible by construction. The revocation list names these ids, and the
rule is: no computed id of any parent key, any child key, or the leaf may appear
in the list.

### 3.6 Canonicality and refusal rules

Exactly one byte string encodes each accepted link. The parser refuses:

1. `link_version != 0x01`, `suite != 0x01`, or any unknown value. Unknown values
   are refusals, never skipped.
2. `description_len > 256`, a truncated or over-long description, or invalid
   UTF-8.
3. `expiry_present` outside `{0x00, 0x01}`, or `expiry_present == 0x00` with a
   non-zero `expiry_millis`.
4. Any trailing byte inside a link after the signature, any partial trailing
   record in the chain, any chain region whose declared length is not consumed
   exactly, more than 8 links, or two links with the same `child_public_key`.
5. Any file where `sig_offset + 64 + zip_len != file_len`, and any file with
   bytes before the header region.
6. Any non-canonical Ed25519 signature. RFC 8032 section 5.1.7 requires decoding
   checks: "If any of the decodings fail (including S being out of range), the
   signature is invalid."
   (`https://www.rfc-editor.org/rfc/rfc8032.txt`). `ed25519-dalek` 3.0.0 must
   not be built with `legacy_compatibility`, described in its own manifest as
   "Turns off stricter checking for scalar malleability in signatures"
   (`https://docs.rs/crate/ed25519-dalek/3.0.0/source/Cargo.toml.orig`).

Size: a one-link chain with a 26-byte description is 135 bytes per link, and the
whole chain for a one-link bundle is smaller than a single minimal X.509
certificate.

## 4. Candidate table

"Established" means the format has a specification that can be read and cited.
Implementation cells name the pinned version, the direct dependencies from the
crates.io dependency API, and the **measured third-party package count** from
`.work/signing-format/deps.md`. Code-size cells are estimates and are labelled as
such.

| Candidate | Link asserts child key | Arbitrary description, bounded | Optional expiry in Unix ms | Key id from public key | Rust implementation, direct deps, measured third-party packages | C or cc build | Verifier code (estimate) | Parser small enough to reimplement? |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| X.509 v3 (RFC 5280, Ed25519 per RFC 8410) | Yes: issuer signs the child's `subjectPublicKeyInfo` | Only through the subject DN or a custom extension; DN has type and encoding rules | No: `notAfter` is UTCTime or GeneralizedTime in whole seconds | Yes, but `subjectKeyIdentifier` is optional and RFC 5280 says applications "are not required to verify that key identifiers match" | Parse: `x509-cert` 0.3.0 (`const-oid`, `der`, `spki`), **13 third-party packages**; `der` 0.8.2 alone, **1**. Validate: `rustls-webpki` 0.103.15 (`rustls-pki-types`, `untrusted`) at **4 packages without a crypto backend, which exports no verification algorithms**, or **8 with `ring`**; `webpki` 0.22.4 at **6 with `ring` mandatory**; `x509-verify` 0.4.8, not measured | No for `x509-cert` and `der`; **yes for `rustls-webpki` and `webpki`**, whose `ring` build script compiles C and assembly through `cc` | Path validation + profile checks: hundreds of lines and the name-comparison rules, before any plugin logic | No: DER plus RFC 5280 section 6 is not a reimplementation target |
| CMS / PKCS#7 (RFC 5652) | Not its job: certs are a bag whose "meaning of a 'certification path' is outside the scope of this document" | Same as X.509 | Same as X.509 | Same as X.509 | `cms` 0.2.3 (`const-oid`, `der`, `spki`, `x509-cert`; optional `aes`, `cbc`, `cipher`, `rsa`, `sha1`, `sha2`, `sha3`, `signature`, `zeroize`); **not measured by the sibling run**, so its tree is at least the 13 of `x509-cert` plus `cms` itself | No | X.509 work plus CMS structure and attribute rules | No |
| JOSE JWS with `x5c` (RFC 7515 section 4.1.6) | Yes, but the chain is X.509: `x5c` "contains the X.509 public key certificate or certificate chain [RFC5280]", and "The recipient MUST validate the certificate chain according to RFC 5280" | No field of its own; a JWS profile would need a custom header plus a JWT-style claim | No: `exp` is a NumericDate in seconds (RFC 7519 section 2) | `kid` is caller-defined; a derived id needs RFC 7638 thumbprints, which is a separate spec and an extra canonical JSON step | `jsonwebtoken` 11.1.0 (`base64`, `ed25519-dalek`, `hmac`, `p256`, `p384`, `pem`, `rand`, `rsa`, `serde`, `serde_json`, `sha2`, `signature`, `simple_asn1`, `zeroize`), **77 third-party packages**, no C build; `josekit` 0.10.3 depends directly on **`openssl`**, which is a C toolchain in the parse path, and was not measured | No for `jsonwebtoken`; **yes for `josekit`** (OpenSSL) | X.509 validation plus JSON and `crit` handling, or a project profile on top of JWS | JWS framing yes, the chain no |
| COSE with `x5chain` (RFC 9360) via `coset` | `x5chain` is "an ordered array of X.509 certificates", so the chain is X.509 again | No COSE header for it; a CWT custom claim would carry it | No: CWT `exp` is NumericDate seconds or float (RFC 8392 section 3.1.4) | `kid` exists as a header parameter; no derivation rule | `coset` 0.4.2 (`ciborium`, `ciborium-io`), **16 third-party packages**, no C build | No | X.509 validation plus CBOR; `coset` itself validates nothing | CBOR yes in principle, the chain no |
| OpenPGP certification (RFC 9580) via `pgp` or `sequoia-openpgp` | Yes: a certification signature over the child key | Yes: user ID or notation subpacket | Only seconds: Key Expiration Time is "the number of seconds after the key creation time that the key expires" | Yes: fingerprint derived from the key | `pgp` 0.21.0, direct deps include `aes`, `aes-gcm`, `aes-kw`, `argon2`, `blowfish`, `camellia`, `cast5`, `cfb-mode`, `curve25519-dalek`, `des`, `dsa`, `ecdsa`, `ed25519-dalek`, `idea`, `k256`, `md-5`, `num-bigint-dig`, `p256`, `p384`, `p521`, `ripemd`, `rsa`, `sha1`, `sha2`, `sha3`, `twofish`, `x25519-dalek`, `zeroize`; **146 third-party packages, the largest measured**, no C build (`bzip2` resolves to the pure-Rust `libbz2-rs-sys`). `sequoia-openpgp` 2.4.2: direct deps `anyhow`, `argon2`, `base64`, `buffered-reader`, `bzip2`, `dyn-clone`, `flate2`, `idna`, `lalrpop-util`, `libc`, `memsec`, `nettle`, `regex`, `regex-syntax`, `sha1collisiondetection`, `thiserror`, `xxhash-rust`; **71 third-party packages normal, 123 under build edges** | **Yes for `sequoia-openpgp`**: `nettle-sys` runs `bindgen`/`clang-sys` with `cc`/`pkg-config` against system nettle and GMP, and a `lalrpop` grammar build script; 52 extra packages behind the build edges | The RFC's layers 1 to 3 plus the application's own semantics layer | No |
| minisign / signify, one signature per link | Yes if the project defines what a "link" signs | Only the trusted comment; the spec page says "arbitrary text" with no stated bound, while the C implementation caps it at 8192 bytes (`TRUSTEDCOMMENTMAXBYTES`) | No expiry field at all | No: `key_id` is "8 random bytes", so relabelling defeats a revocation list | Sign: `minisign` 0.10.0 (`ct-codecs`, `getrandom`, `rpassword`, `scrypt`), **21 third-party packages**. Verify: `minisign-verify` 0.3.0, **1 third-party package**, tied for the smallest measured | No | Small, but the link record is invented anyway | The container is text and base64; the link is not specified by anyone |
| PASETO v4 | No chain primitive: one signature by one key, with purposes `local` and `public` only | Application-defined payload or footer | No: the specification's example payload shows `exp` as an RFC 3339 string | No | `pasetors` 0.8.1 (`ct-codecs`, `ed25519-compact`, `getrandom`, `orion`, `regex`, `serde_json`, `subtle`, `time`, `zeroize`), **24 third-party packages**, no C build | No | n/a, no chain | Container yes, chain does not exist |
| TUF metadata | Role delegation, not a per-link child-key assertion | No | No: `expires` is "YYYY-MM-DDTHH:MM:SSZ" | Yes: `keyid` "is a hexdigest of the SHA-256 hash of the canonical form of the key" | `tough` 0.24.0 pulls `aws-lc-rs`, `rustls`, `tokio`, optional `reqwest` (section 3.9 of the earlier findings); **not measured by the sibling run**; `aws-lc-rs` is a C and assembly build through `aws-lc-sys` | **Yes** (`aws-lc-rs`) | The full client workflow: versions, thresholds, freeze checks | No: canonical JSON plus role semantics |
| **Project format (recommended)** | Yes, by construction | Yes, `description_len` and a 256-byte cap | Yes, `i64` milliseconds | Yes, `SHA-256(public key)[0..16]`, recomputed by the verifier | `ed25519-dalek` 3.0.0 verify-only (`default-features = false`: `curve25519-dalek`, `ed25519`, `sha2`, `subtle`), **18 third-party packages**, no C build; `sha2` and `subtle` are already in the workspace | No | One fixed-offset walk with bounds checks, no recursion and no allocation beyond the description: tens of lines of parsing plus the policy | Yes: the layout above is the whole format |

### 4.1 Measured cost, as taken by the sibling run

All figures are third-party packages from
`.work/signing-format/deps.md`, measured with
`cargo tree --edges normal` (and `--edges build,normal` for the build column) on
`.work/signing-format/deps/`, sorted smallest first.

| Measured crate and configuration | Third-party packages | C or `cc` build |
| --- | --- | --- |
| `der` 0.8.2, default | 1 | no |
| `minisign-verify` 0.3.0 | 1 | no |
| `rustls-webpki` 0.103.15, default only, no backend | 4 | no, but it exports no verification algorithms, so it cannot verify anything |
| `ed25519-compact` 2.6.0, default | 5 | no |
| `webpki` 0.22.4 | 6 | **yes**: `ring` is a mandatory dependency and its build script compiles C and assembly through `cc` |
| `rustls-webpki` 0.103.15 with `ring` | 8 | **yes**, same `ring` build |
| `x509-cert` 0.3.0, default | 13 | no |
| `coset` 0.4.2, default | 16 | no |
| `ed25519-dalek` 3.0.0, `default-features = false` | 18 | no |
| `minisign` 0.10.0, default | 21 | no |
| `pasetors` 0.8.1, default | 24 | no |
| `sequoia-openpgp` 2.4.2, default `crypto-nettle` | 71, or 123 under build edges | **yes**: `nettle-sys` with `bindgen`, `clang-sys`, `cc`, `pkg-config` |
| `jsonwebtoken` 11.1.0, `use_pem` + `rust_crypto` | 77 | no |
| `pgp` 0.21.0, default | 146 | no, but the tree carries a second RustCrypto generation (`der` 0.7, `digest` 0.10) |

Two notes that matter for a parse path that leads to loading compiled code: the
three certificate-validating options that are actually usable hide a C build
behind a mandatory crypto backend (`ring` for `rustls-webpki` and `webpki`,
`aws-lc-rs` for the unmeasured `tough`), and the recommended path is the only one
at this level that is both small and C-free.

## 5. Candidate notes

### 5.1 X.509

Direct evidence for the fields. The subject and its alternatives: "The subject
field identifies the entity associated with the public key stored in the subject
public key field. The subject name MAY be carried in the subject field and/or the
subjectAltName extension"
(`https://www.rfc-editor.org/rfc/rfc5280.txt`, section 4.1.2.6). Validity:
"Both notBefore and notAfter may be encoded as UTCTime or GeneralizedTime", the
values "MUST include seconds", and "GeneralizedTime values MUST NOT include
fractional seconds" (sections 4.1.2.5, 4.1.2.5.1, 4.1.2.5.2). So milliseconds
are not expressible without a convention that the format does not define. Key
ids: two methods are given, method 1 being "the 160-bit SHA-1 hash of the value
of the BIT STRING subjectPublicKey", and the spec adds that "Applications are not
required to verify that key identifiers match when performing certification path
validation" (section 4.2.1.2), so a key id read from a certificate is a hint, not
a checked value. Ed25519 is legal in a certificate: `id-Ed25519 OBJECT IDENTIFIER
::= { 1 3 101 112 }`, with the key as "the byte stream of the public key" in the
`subjectPublicKey` BIT STRING, and if `keyUsage` is present in an end-entity
certificate it "MUST contain one or both of the following values: nonRepudiation;
and digitalSignature" (`https://www.rfc-editor.org/rfc/rfc8410.txt`, sections 4
and 5). None of that profile machinery has meaning for a plugin signing chain.

The cost is the validator, and it is not a small crate. `x509-cert` is described
as a "Pure Rust implementation of the X.509 Public Key Infrastructure Certificate
format"; its documented modules are `anchor`, `attr`, `builder`, `certificate`,
`crl`, `ext`, `name`, `request`, `serial_number`, `time`, with no validator
(`https://docs.rs/x509-cert/latest/x509_cert/`). It measures 13 third-party
packages and no C build. `rustls-webpki` does validate, but its own README lists
what it does not offer: "Support for self-signed certificates", "Access to
arbitrary certificate extensions", "Parsing/representation of certificate
subjects, or human-friendly display of these fields"
(`https://github.com/rustls/webpki`). **Researcher inference:** those three
omissions are exactly the S8 description field, so a webpki-based verifier could
validate a chain and still be unable to read what each link says, which means a
second parsing stack (the suggested `x509-parser`) would be needed for the
description, and the two stacks then have to agree about the same bytes. Its
measured cost is 4 packages with no crypto backend, which cannot verify
anything, or 8 packages with `ring`, which adds a C and assembly build step to a
security-critical parse path. The original `webpki` crate is frozen at 0.22.4
(`https://crates.io/api/v1/crates/webpki`), depends on `ring` unconditionally
(6 packages plus the same C build), and `rustls-webpki` documents itself as "a
fork of the original webpki project". The separate RustCrypto-family validator
carries its own warning: "Some of the features of this crate are in an early,
experimental phase. Use at your own risk."
(`https://docs.rs/crate/x509-verify/latest`), and its documented usage verifies a
certificate's signature given a supplied key, without the section 6 path rules.

Hostile input, DER side: RFC 5280 mandates refusal on "a critical extension it
does not recognize or a critical extension that contains information that it
cannot process", and requires that an application "MUST either process the
constraint or reject the certificate" for a critical name constraint, and "MUST be
able to process name constraints that are imposed on the directoryName name form"
(sections 4.2, 4.2.1.10). Name chaining is defined by section 7.1 with
string-type and internationalization rules. The `der` crate is the right kind of
dependency here, since it is deliberately strict and is the smallest measured
crate at 1 third-party package: "The DER decoder in this crate performs checks to
ensure that the input document is in canonical form, and will return errors if
non-canonical productions are encountered. There is currently no way to disable
these checks." (`https://docs.rs/der/latest/der/`). That removes BER ambiguity,
but every rule above remains the project's to implement.

Verdict: rejected. It is established and it can carry a chain, but two of the
three S8 fields need invented encodings inside it, the expiry granularity is
wrong, and the validation code the format requires is larger and subtler than the
whole project format, while the one small usable validator cannot read the
description and drags in a C build.

### 5.2 CMS and PKCS#7

`SignedData` carries `certificates [0] IMPLICIT CertificateSet OPTIONAL` and
`signerInfos SignerInfos`, where `SignerInfos ::= SET OF SignerInfo`, and "There
MAY be any number of elements in the collection, including zero". The certificate
bag is explicitly not a path: "It is intended that the set of certificates be
sufficient to contain certification paths from a recognized 'root' or 'top-level
certification authority' to all of the signers", while "The precise meaning of a
'certification path' is outside the scope of this document." `SignerInfo` adds
`digestAlgorithm`, `signedAttrs [0] IMPLICIT SignedAttributes OPTIONAL`,
`signatureAlgorithm`, `signature`, and `unsignedAttrs [1] IMPLICIT
UnsignedAttributes OPTIONAL`, where unsigned attributes are by definition not
signed and the documented example use is "countersignatures"
(`https://www.rfc-editor.org/rfc/rfc5652.txt`, sections 5.1, 5.3, 10.2.3, 11.4).
Hostile input: the message digest is taken over a re-encoded form of the
`signedAttrs` value, with the IMPLICIT tag replaced by an EXPLICIT SET OF tag
(section 5.4), which means the verifier reconstructs bytes rather than hashing
what it received. Rejected: strictly more surface than X.509, no chain semantics
of its own, and a multi-signer, multi-attribute model the project has no use for.
The Rust `cms` 0.2.3 crate adds `x509-cert` and keeps the parser, not the policy;
it was not in the sibling measurement, so no package count is claimed for it.

### 5.3 JOSE JWS with `x5c`

Direct evidence: `x5c` "contains the X.509 public key certificate or certificate
chain [RFC5280] corresponding to the key used to digitally sign the JWS",
represented as a JSON array of base64 (not base64url) DER values; "The
certificate containing the public key corresponding to the key used to digitally
sign the JWS MUST be the first certificate. This MAY be followed by additional
certificates, with each subsequent certificate being the one used to certify the
previous one. The recipient MUST validate the certificate chain according to RFC
5280 [RFC5280] and consider the certificate or certificate chain to be invalid if
any validation failure occurs." The RFC also notes that key-identifying headers
"MUST be integrity protected if the information that they convey is to be
utilized in a trust decision"
(`https://www.rfc-editor.org/rfc/rfc7515.txt`, sections 4.1.6 and 6). JOSE has
`kid` and `crit`; it has no description and no expiry outside JWT claims, whose
`exp` is "the number of seconds from 1970-01-01T00:00:00Z UTC", explicitly
allowing non-integer values (`https://www.rfc-editor.org/rfc/rfc7519.txt`,
section 2).

Two ways to use it, both rejected. (a) `x5c` means X.509 validation, as the RFC
itself requires, so it is X.509 plus JSON, and the measured JOSE option that is
pure Rust is 77 third-party packages for token handling alone, before any chain
validation. (b) Links as JWS objects that assert a key: the nearest standardized
parts are the confirmation claim of RFC 7800 and the RFC 7638 JWK thumbprint as a
derived id, but the delegation record is then the project's invention, so the
project pays JSON, base64, `crit`, and JWK canonicalization for a format it still
has to specify. The other direct Rust option, `josekit` 0.10.3, depends directly
on `openssl`, which puts a C library in the parse path.

### 5.4 COSE, `x5chain`, `coset`

Direct evidence: `x5chain` "contains an ordered array of X.509 certificates. The
certificates are to be ordered starting with the certificate containing the
end-entity key followed by the certificate that signed it, and so on."
(`https://www.rfc-editor.org/rfc/rfc9360.html`). So `x5chain` is X.509, and
`coset` provides types (`CoseKey`, `CoseKeySet`, `CoseSign1`, `Header`) and
`sig_structure_data`, but its documented type list contains no typed `x5chain`
handling and no certificate validation
(`https://docs.rs/coset/latest/coset/`). CWT claims are the only expiry: `exp`
uses the JWT rules with a CBOR NumericDate, an integer or floating-point value
in seconds (`https://www.rfc-editor.org/rfc/rfc8392.txt`, sections 2 and 3.1.4).
CBOR itself is a fixed encoding for the signed structure: "Encoding MUST be done
using definite lengths, and the length of the (encoded) argument MUST be the
minimum possible length" (`https://www.rfc-editor.org/rfc/rfc9052.txt`, section 9),
but `coset`'s own documentation shows the trap that matters here: after a
successful verification, "Changing an unprotected header leaves the signature
valid" and the assertion `verify_signature(...).is_ok()` still holds. Hostile
input therefore includes a link whose description or key is placed in an
unprotected header and is not covered by the signature at all. Rejected as a
chain carrier; kept as the least-costing established container if the operator
ever wants one, since `coset` 0.4.2 measures 16 third-party packages with no C
build.

### 5.5 OpenPGP

Direct evidence for the fields: key ids are derived from the key material (the
fingerprint is defined in section 3.3 of the RFC), a certification signature
expiry and a key expiry are 4-octet time fields, and "The validity period of the
key. This is the number of seconds after the key creation time that the key
expires" (`https://www.rfc-editor.org/rfc/rfc9580.txt`, sections 3.3, 5.2.3.13,
5.2.3.18). So description and expiry exist, but in seconds, and behind an
unbounded set of signature types.

The disqualifier is stated by the implementation itself: "Of these layers, the
OpenPGP RFC specifies 1-3, while 4 is not specified in detail", where layer 4 is
"OpenPGP semantics (e.g.: Expiration and Revocation of Certificates and their
components, Key Flags ...)"; "rPGP handles layers 1-3, but explicitly does not
deal with 4. Applications that need OpenPGP semantics must implement them
manually", and "the APIs are low level building blocks and do not claim that
using them are (a) secure or (b) following the OpenPGP specification"
(`https://docs.rs/crate/pgp/latest`). Layer 4 is exactly the part the operator
asked about.

The measured cost is decisive as well: `pgp` 0.21.0 is the largest tree in the
measurement at **146 third-party packages**, carrying a full cryptography suite
(AES, Camellia, DES, IDEA, Twofish, Blowfish, RSA, DSA, ECDSA, EdDSA, Argon2,
SHA-1, SHA-2, SHA-3, curve25519, x25519) plus a second RustCrypto generation
(`der` 0.7, `digest` 0.10); it has no C build, because `bzip2` resolves to the
pure-Rust `libbz2-rs-sys`. `sequoia-openpgp` 2.4.2 is 71 packages normally and
**123 under build edges**; it builds the C `nettle` library through `nettle-sys`,
`bindgen`, and `clang-sys`, with `cc` and `pkg-config`, and runs a `lalrpop`
grammar build. Neither is a candidate for a small trusted parser.

### 5.6 minisign or signify, once per link

Direct evidence: the signature file is `untrusted comment`,
`base64(signature_algorithm || key_id || signature)`, `trusted comment`,
`base64(global_signature)`, and the public key file is `untrusted comment`,
`base64(signature_algorithm || key_id || public_key)`; the signing input is the
file data, or its BLAKE2b-512 prehash in hashed mode; and "key_id: 8 random
bytes"
(`https://jedisct1.github.io/minisign/`). The reference implementation's header
defines `TRUSTEDCOMMENTMAXBYTES 8192` and `COMMENTMAXBYTES 1024`, so the
"arbitrary text" has a de facto bound that the specification page does not state
(`https://raw.githubusercontent.com/jedisct1/minisign/master/src/minisign.h`).
There is no expiry field. A per-link design must therefore define what bytes are
signed (an encoded child key plus description plus expiry), which is a project
format wearing minisign as its signature envelope, and it keeps minisign's random
key id, which is the one property S7 was written to avoid. The measured numbers
are the brightest spot of this candidate: `minisign-verify` 0.3.0 is 1
third-party package, tied for smallest, and `minisign` 0.10.0 is 21, neither with
a C build. That keeps minisign attractive for the scheme decision already taken,
where the earlier findings recommend it, but not as a chain carrier.

### 5.7 PASETO

PASETO v2 is deprecated and v4 is current; `Version4.md` defines exactly two
operations, `v4.local` and `v4.public`, with one signature over
`PAE(header, payload, footer, implicit assertion)` and base64url without padding
(`https://raw.githubusercontent.com/paseto-standard/paseto-spec/master/docs/01-Protocol-Versions/Version4.md`,
`.../Version2.md`, `.../Common.md`). The specification's own example payload
shows a claim set with `exp` as an RFC 3339 timestamp string
(`https://raw.githubusercontent.com/paseto-standard/paseto-spec/master/README.md`).
**Researcher inference:** a token format with one signer per token, no key id,
no certificate, and an application-defined payload is not a chain carrier; using
it would mean defining a link payload and a footer convention, i.e. the project
format again, with a JSON payload and a base64 layer added. Its measured cost of
24 third-party packages and no C build does not change that. Rejected.

### 5.8 TUF metadata

Direct evidence: `root.json` maps `KEYID` to `KEY`, with `KEYID` defined as the
identifier of the key "which is a hexdigest of the SHA-256 hash of the canonical
form of the key", and `expires` as "A date-time string ... Clients MUST NOT trust
an expired file", where the date-time format is "YYYY-MM-DDTHH:MM:SSZ"
(`https://theupdateframework.github.io/specification/latest/`, sections 4.2.2,
4.2.3, 4.3). The key id is therefore derived from the key, which satisfies S7, and
the expiry is a seconds-resolution string, which does not satisfy S8. There is no
per-link description field. The format also requires a general principle the
project would have to honour: implementations must "choose a data format that
allows for canonicalization, or one that will decode data deterministically by
default so that signatures can be accurately verified", with the specification's
own examples using a "canonical JSON" subdialect. Rejected on shape and on cost,
as already recorded in section 3.9 of the earlier findings; its Rust client
`tough` was not in the sibling measurement, and its `aws-lc-rs` dependency is a C
and assembly build.

## 6. Hostile input analysis

The parser's job is not only to refuse bad bytes. Because the payload signature
covers the same bytes that the loader, the ZIP reader, and the chain parser each
interpret, any disagreement between those readers is a forgery seam. The parser
must therefore have exactly one in-memory result per accepted byte string, and
"accept and ignore" must not exist for anything a trust decision reads.

Cross-cutting hazards, and how the recommended layout answers them:

| Hazard | Recommended layout |
| --- | --- |
| Length fields that overflow or exceed the buffer | One 2-byte length, capped at 256, checked against the remaining chain bytes before any read |
| Trailing or embedded extra data | The chain length must be consumed exactly, every link ends at its own signature, and `sig_offset + 64 + zip_len == file_len` |
| Ambiguous encodings of the same value | Fixed-width fields, fixed stride, one encoding for absent description and absent expiry, unknown version or suite refused |
| Recursion or nesting | None: a flat array of equally shaped records, capped at 8 |
| Allocation blowup | One bounded 256-byte description per link; the rest is fixed-size |
| Type confusion between a key and a signature | Offsets are fixed, and `suite` pins the 32-byte key and 64-byte signature sizes |
| Signature malleability | RFC 8032 decoding checks; no `legacy_compatibility` in `ed25519-dalek` |
| Cross-protocol signature reuse | Distinct domain tags for the link and the payload inputs |
| Key relabelling to defeat revocation | Ids are recomputed from the public key and never read from the wire |
| Time parsing and clock skew | One `i64` field, no calendar arithmetic, no leap-second rule, no time zone |
| Build-time trust in the parse path | No C compiler and no `cc` build script anywhere in the recommended path |

Per candidate, the hostile input that the project would inherit:

- **X.509**: the largest set. Non-canonical DER is refused by `der`, but the
  project still owns unknown-critical-extension refusal, the name-constraint
  rule, section 7.1 name comparison, `basicConstraints` and `keyUsage` profile
  checks, the UTCTime/GeneralizedTime 2049 boundary, serial-number sign rules,
  and the negative-notBefore/notAfter cases. `rustls-webpki` reduces that set but
  also hides the description field, which forces a second parser over the same
  bytes, and it adds a `ring` C and assembly build through `cc` to the trusted
  path for the privilege.
- **CMS**: the X.509 set plus multi-signer sets, "zero signers" (which the
  specification permits), digest-algorithm consistency between
  `digestAlgorithms` and `SignerInfo`, and a digest computed over a re-encoded
  attribute set. Unprotected attributes are not covered by the signature and a
  verifier that reads them for a trust decision has a hole.
- **JOSE**: JSON duplicate keys, number and string coercions, base64 versus
  base64url, `crit` parameters that must be honoured or refused, and the
  unprotected-versus-protected header split. If `x5c` is used, every X.509
  hazard above arrives with it.
- **COSE**: CBOR indefinite lengths, tags, integer-width variation, duplicate map
  keys, and deep nesting. `coset`'s documented behaviour that an unprotected
  header change leaves the signature valid is the specific trap.
- **OpenPGP**: partial-length packets, armor and cleartext framing, MPI and
  subpacket parsing, multiple and conflicting self-signatures, and an expiry and
  revocation semantic layer the RFC leaves unspecified, which the implementation
  documentation confirms it does not provide. `sequoia-openpgp` additionally
  puts `bindgen`, `clang-sys`, `cc`, and `pkg-config` in the build, against system
  nettle and GMP.
- **minisign / signify**: the parser is simple and the verifier is one package,
  but the trusted comment bound is an implementation constant, not a
  specification rule, and the key id is not covered by the signature.
- **PASETO**: JSON payload claims, base64url strictness (trailing-bit clearing is
  mandated by `Common.md`), algorithm-lucidity checks in the implementation
  guide, and a footer the specification lets an implementation treat as optional
  to compare.
- **TUF**: canonical JSON (the specification recommends a canonicalizing data
  format and cites a non-IETF canonical JSON definition), role and threshold
  logic, and a version-and-expiry workflow designed for a network repository.

## 7. The five tamper cases

Direct evidence for the coverage rule: the earlier findings establish that a
whole-file Ed25519 signature detects a modified manifest, a swapped library, an
added entry, a removed entry, and truncation, and S1 moves that rule inside the
file: the payload signature covers the header, the chain and the ZIP bytes.

**No chain format in section 4 covers the payload by itself.** Every candidate
carries a chain and, at most, its own internal structure. The five cases are
therefore decided by the S1 payload signature, not by the choice in this note,
and the answer is the same for all candidates:

| Tamper case | Detected by | Result for every candidate |
| --- | --- | --- |
| Modified `manifest.toml` | S1 payload signature over the ZIP bytes | Yes |
| Swapped library | S1 payload signature | Yes |
| Added archive entry | S1 payload signature | Yes |
| Removed archive entry | S1 payload signature | Yes |
| Truncated file | S1 payload signature plus `sig_offset + 64 + zip_len == file_len` | Yes |

If instead a candidate's own coverage idiom were used for the payload (a signed
entry, a signed digest manifest, an APK-style block), the earlier findings
already record the exceptions: a JAR-style digest manifest needs an added-entry
rule that Android had to add and a removed-entry rule that the JAR specification
does not state, and it leaves ZIP headers, extra fields, comments and ordering
outside the signed set; the APK block detects all five but costs the two-level
digest, offset normalization, and the "no data after the end of central
directory" packaging rule. Both were rejected there on cost.

Where the candidates genuinely differ is link-level tampering, which is what this
note decides:

| Candidate | Reorder links | Splice a link from another bundle | Change a description | Change or remove an expiry | Swap a child key | Drop or add a link |
| --- | --- | --- | --- | --- | --- | --- |
| X.509 | Refused if the verifier checks name chaining; a reordering that still chains is accepted | Accepted if the spliced certificate is genuinely issued by the same parents | Accepted if the reader trusts the DN or extension without checking the issuer's signature over it, which is the default reading of a DN | `notAfter` is inside the signed TBS, so refused | Refused | Chain length must be checked; a shorter chain to the same root is accepted by default |
| CMS | Same as X.509 | Same as X.509 | Same as X.509 | Same as X.509 | Refused | Same as X.509 |
| JOSE with `x5c` | Same as X.509 | Same as X.509 | Same as X.509 | Same as X.509 | Refused | Same as X.509 |
| COSE with `x5chain` | Same as X.509 | Same as X.509 | Same as X.509 | Same as X.509 | Refused | Same as X.509 |
| OpenPGP | Depends on which signatures the project's own semantics layer counts; multiple certifications can coexist | Accepted unless the application prefers one signature and ignores the rest | Depends on the semantics layer | Depends on the semantics layer, because the expiry is in a self-signature | Refused | Depends on the semantics layer |
| minisign per link | Refused, each link is a separate signature | Accepted: signatures over the same child key are interchangeable, and the key id is not signed | Accepted: the trusted comment is signed, but nothing binds it to a slot in a chain | There is no expiry | Refused | Accepted |
| PASETO | Not applicable | Not applicable | Not applicable | Not applicable | Not applicable | Not applicable |
| TUF | Delegation structure is signed, so reordering is refused | Refused by role and version rules | No description field | Refused: expiry is inside the signed metadata | Refused | Refused by version and hash rules |
| **Project format** | Refused: link `i` binds the key that signs link `i + 1` | Refused: a spliced link must still be signed by the previous link's child key, so it can only be replayed verbatim into a chain with the same keys, and a duplicate child key is refused | Refused: the description is inside the signed link input | Refused: the expiry is inside the signed link input | Refused | Refused: the chain's byte length is declared and must be consumed exactly, and any link count change changes the signed input |

## 8. Rejected candidates and the reason each fails

1. **X.509 v3 certificates.** Established, chain capable, and it does assert the
   child public key, but: the description has no field of its own (subject DN or
   custom extension, both project-defined); the expiry is whole seconds and there
   is no millisecond form; the key id extension is optional and RFC 5280 does not
   require a validator to verify it; and the validation code that makes the
   format worth adopting is not available as a reviewed component
   (`x509-cert` has no validator at 13 packages, `rustls-webpki` cannot expose
   the fields the chain must carry and needs a `ring` C build at 8 packages,
   `x509-verify` calls itself experimental). The project would write a path
   validator for a two-or-three-link chain of self-issued keys.
2. **CMS and PKCS#7.** Adds ASN.1 surface over X.509, defines no path semantics
   ("The precise meaning of a 'certification path' is outside the scope of this
   document"), permits zero or many signers, computes its digest over re-encoded
   attributes, and keeps a class of unsigned attributes that a careless verifier
   might read. Every X.509 objection applies, plus more. Not measured by the
   sibling run.
3. **JOSE JWS with `x5c`.** `x5c` is X.509 and the RFC requires RFC 5280
   validation of it, so the established path is X.509 plus JSON. The alternative,
   a link as a JWS with a confirmation key, is not an established chain format:
   it is a composition the project would specify, at the cost of JSON, base64,
   `crit`, and JWK canonicalization. The pure-Rust option measures 77 third-party
   packages, and the other one, `josekit`, links OpenSSL.
4. **COSE with `x5chain` through `coset`.** `x5chain` is X.509 again, `coset`
   exposes no typed `x5chain` field and no certificate validation, CWT expiry is
   seconds, and `coset`'s own documentation shows that unprotected header changes
   leave the signature valid. Its measurement, 16 third-party packages and no C
   build, is the best of any established candidate, but the payload is still
   X.509.
5. **OpenPGP through `sequoia-openpgp` or `pgp`.** The format has derived key ids
   and both description and expiry, but the expiry is in seconds, the semantics
   layer the operator needs (expiration and revocation) is outside the
   specification by the RFC's own split and by the implementation's own
   statement, and the measured cost is 146 third-party packages for `pgp` 0.21.0
   and 71 packages plus a `bindgen`/`clang`/`nettle` C build at 123 packages for
   `sequoia-openpgp` 2.4.2.
6. **minisign or signify once per link.** The key id is 8 random bytes, so a
   revocation list can be sidestepped by relabelling, which S7 exists to prevent.
   There is no expiry field, the trusted comment's bound is an implementation
   constant rather than a specification rule, and the project would have to define
   what a link signs, so the format does not save the work it appears to save.
   This does not disturb the earlier findings' scheme recommendation:
   `minisign-verify` at 1 package and no C build remains the cheapest verifier
   for a flat detached signature.
7. **PASETO.** Not a chain format: one signature per token, two purposes, no key
   id, no certificate, and an application-defined payload. Using it means
   defining the link and a footer convention, i.e. the project format with JSON
   and base64 added. Its cost, 24 packages and no C build, does not change that.
8. **TUF metadata.** The permission model is repository roles, the flow assumes
   downloads with version, threshold, and freeze checks, canonical JSON is a
   requirement the project would have to satisfy, the expiry is a
   seconds-resolution string, there is no per-link description, and its Rust
   client brings `aws-lc-rs`, which is a C and assembly build.

## 9. Why the format question ends in a project format

The argument is not that the established formats are bad. It is that the three
S8 fields are a conjunction that none of them native-carries: a chain link with a
parent signature over the child's key, an arbitrary bounded description, and an
optional expiry in Unix milliseconds. Each candidate fails at least one of the
three, and each failure is repaired only by defining a profile: an extension OID
or a DN convention in X.509, a custom CWT claim in COSE, a notation or user ID in
OpenPGP, a trusted comment in minisign. A profile is a project format, and the
established container around it then costs a parser, a dependency tree, or a
validator that the project must still write.

Three further facts decide the tie. First, the formats that do carry a bounded
arbitrary field and an expiry use seconds, because every general-purpose PKI is
built for calendar time (RFC 5280 says GeneralizedTime values "MUST NOT include
fractional seconds"; RFC 9580 defines key expiration as "the number of seconds
after the key creation time"; TUF's date-time is "YYYY-MM-DDTHH:MM:SSZ"). The
operator asked for milliseconds, which is a project requirement no candidate
meets. Second, the measured cost of the trusted path favours a small project
format: verify-only `ed25519-dalek` is 18 third-party packages with no C build,
and `sha2` and `subtle` are already in the workspace, while 14 of the 15
alternatives measured at or above `coset`'s 16 packages either add a validator
the project must write or a C build step, and the two OpenPGP routes are 71 and
146 packages. Third, a DER, CBOR, JSON, or OpenPGP parser cannot be reimplemented
safely by this project, which is exactly the operator's stated second constraint,
and its acceptance leads to loading compiled code.

## 10. Unresolved questions the operator must settle

1. **Does an expired link abort, or degrade, and with what clock-skew
   tolerance?** S6 made a failed signature fatal; expiry is still open in
   `plugin-signing`. Recommendation: if expiry is enforced, allow a documented
   skew (a few minutes) and treat expiry as fatal, because a mixed outcome for
   two integrity failures is harder to reason about. The format carries the field
   either way.
2. **Is the description checked against anything, or informational only?** S8
   says the operator prefers a free description, so the verifier cannot check it.
   If the description is informational, the contract text should say the verifier
   MUST NOT make a trust decision from it, so no future implementation starts
   matching substrings.
3. **Who holds the parent keys, and how does a chain get built in practice?**
   S4 says the signing job holds the signing key and the chain. The layout above
   requires the signer to hold every parent key in the chain, or to receive
   already-signed links. The SDK CLI's input shape is affected.
4. **Is the chain limited to one link in v1?** The layout allows up to 8 and the
   S9 decision leaves the compiled root set empty with tests carrying their own,
   so a one-link chain is the only exercised case today. If the operator wants to
   promise "chain of one" for v1, the format should still accept the general
   shape and document the limit.

Two smaller points that need only a nod: whether the payload-signature offset is
declared in the header or implied by the header's fixed size (S1 says declared,
and this note assumes declared), and whether `metis-plugin-chain-link-v1` and
`metis-plugin-payload-v1` are the final domain tags or should carry the bundle
format version as well.

## 11. Sources

Kept:

- `.work/signing-format/deps.md`: every measured dependency number in this note,
  taken with `cargo tree --edges normal` and `--edges build,normal` on the
  scratch workspace `.work/signing-format/deps/`, including the third-party
  package counts and the list of build-script packages that pull a C compiler.
- `https://www.rfc-editor.org/rfc/rfc5280.txt`: the subject field, the validity
  encodings and the no-fractional-seconds rule, the two key-identifier methods
  and the statement that applications are not required to verify them, the
  unknown-critical-extension refusal, the name-constraint processing rule, and
  the section 6 validation steps.
- `https://www.rfc-editor.org/rfc/rfc8410.txt`: Ed25519 in X.509, the
  `id-Ed25519` OID, the public key as a byte string in `subjectPublicKey`, and the
  key-usage rules for end-entity and CA certificates.
- `https://www.rfc-editor.org/rfc/rfc5652.txt`: the `SignedData` and `SignerInfo`
  structures, the certificate bag's status as a non-path, the zero-or-many
  signers rule, the re-encoded `signedAttrs` digest input, and the unsigned
  attributes.
- `https://www.rfc-editor.org/rfc/rfc7515.txt`: the `x5c` definition, the DER
  base64 encoding of the chain, and the requirement to validate the chain per RFC
  5280.
- `https://www.rfc-editor.org/rfc/rfc7519.txt`: `NumericDate` as seconds since the
  epoch, the `exp` encoding.
- `https://www.rfc-editor.org/rfc/rfc9052.txt`: the COSE header split and the
  minimum-length, definite-length encoding restriction.
- `https://www.rfc-editor.org/rfc/rfc9360.html`: `x5chain` as an ordered array of
  X.509 certificates, end-entity first.
- `https://www.rfc-editor.org/rfc/rfc8392.txt`: the CWT `exp` claim as a
  NumericDate with integer or floating-point values.
- `https://www.rfc-editor.org/rfc/rfc9580.txt`: OpenPGP key ids and fingerprints,
  the Key Expiration Time subpacket as seconds after key creation, and the
  signature expiration subpacket.
- `https://www.rfc-editor.org/rfc/rfc8032.txt`: the Ed25519 verification rule
  that rejects failed point decodings and out-of-range `S`; already cited in the
  earlier findings, re-used here for the malleability refusal rule.
- `https://theupdateframework.github.io/specification/latest/`: the keyid as the
  SHA-256 hash of the canonical form of the key, the date-time format, the
  no-canonicalization principle, role keys, and thresholds.
- `https://jedisct1.github.io/minisign/`: the signature and public key file
  layout, the 8 random key-id bytes, the trusted and untrusted comments, the
  prehash rule, and the signing input.
- `https://raw.githubusercontent.com/jedisct1/minisign/master/src/minisign.h`:
  `TRUSTEDCOMMENTMAXBYTES 8192` and `COMMENTMAXBYTES 1024`, the de facto comment
  bounds.
- `https://docs.rs/der/latest/der/`: the decoder's canonical-form checks and the
  statement that they cannot be disabled.
- `https://docs.rs/x509-cert/latest/x509_cert/`: the crate's module list, which
  contains no validator, and its self-description as a format implementation.
- `https://docs.rs/crate/x509-verify/latest`: the experimental-phase warning and
  the documented verification API.
- `https://docs.rs/crate/rustls-webpki/latest` and
  `https://github.com/rustls/webpki`: the limitations list, including no access
  to arbitrary certificate extensions and no subject parsing, and the fork
  statement.
- `https://crates.io/api/v1/crates/webpki`: the original crate is at 0.22.4.
- `https://docs.rs/crate/coset/latest` and `https://docs.rs/coset/latest/coset/`:
  the type list, the `sig_structure_data` helper, and the demonstration that an
  unprotected header change leaves the signature valid.
- `https://docs.rs/crate/pgp/latest`: the four-layer description of OpenPGP, the
  statement that layer 4 is not specified and not implemented, and the low-level
  API warning.
- `https://raw.githubusercontent.com/paseto-standard/paseto-spec/master/docs/01-Protocol-Versions/Version4.md`,
  `.../Version2.md`, `.../Common.md`, and the repository README: the two purposes,
  the PAE construction, the strict base64url rules, the application-defined
  payload, and the RFC 3339 `exp` in the specification's example.
- Direct-dependency evidence from the crates.io dependency API, one call per
  crate and version: `x509-cert` 0.2.5, `cms` 0.2.3, `coset` 0.4.2, `pgp` 0.21.0,
  `sequoia-openpgp` 2.4.1, `pasetors` 0.8.1, `josekit` 0.10.3, `jsonwebtoken`
  10.1.0, and `rustls-webpki` 0.103.15. Where the sibling measurement pinned a
  newer version (`x509-cert` 0.3.0, `sequoia-openpgp` 2.4.2, `jsonwebtoken`
  11.1.0), the measured version governs.
- Local: `docs/tasks/metis-plugins/tasks/plugin-signing/task.md` for S1 to S10,
  `docs/tasks/metis-plugins/tasks/plugin-signing-research/findings.md` for the
  scheme comparison, the five tamper cases, and the JAR and APK coverage rules,
  `docs/plugin-contract.md` for the ABI and archive decisions, `Cargo.toml` and
  `crates/plugin-runtime/Cargo.toml` for the dependency budget and the
  `jsonschema` refusal comment, and `docs/tasks/metis-plugins/tasks/plugin-signing-format/task.md`
  for the required evidence.

Rejected or deprioritized:

- SEO and encyclopedia pages on X.509, CBOR, and ZIP: superseded by the RFCs and
  the crate documentation above.
- Non-version-pinned blog posts on JWT and COSE pitfalls: the RFCs and the crate
  documentation state the same points with authority.
- The `openssh-portable` `PROTOCOL.certkeys` file: the raw URL returns HTTP 404,
  so the OpenSSH certificate format is not assessed here (see missing evidence).
- Secondary copies of RFC 5280 and RFC 9580: the rfc-editor.org texts were read
  directly.
- `x509-parser` docs: cited only as the alternative `rustls-webpki` itself
  recommends; its dependency surface was not measured because this candidate was
  rejected on shape, not on cost.

## 12. Confidence and limits

- Confidence in the recommendation and in the layout is **high**: the layout is
  derived from the settled scheme, and every refusal rule maps to a named hazard.
- Confidence in the "no established candidate carries all three S8 fields" claim
  is **medium to high**. It is grounded in direct quotations for each field of
  each candidate, but it is a negative claim about design space: a profile of an
  established format could always be defined. The argument here is that a profile
  is a project format, so the profile route loses the property that motivated
  using an established format, which is reviewer familiarity with the bytes.
- Confidence in the dependency figures is **high**: direct dependencies come from
  the crates.io dependency API at pinned versions, and third-party package counts
  and build-step findings come from the sibling `cargo tree` measurement at
  `.work/signing-format/deps.md`. Three candidates are outside that measurement
  (`cms`, `josekit`, `tough`), and for those this note claims only their
  crates.io direct dependencies and marks the count as unmeasured.
- Confidence in the code-size estimates is **low**, and they are labelled as
  estimates. No code was written and no crate was built for this note.
- **Validation limitation:** one `source_check` call was made, on the
  `rustls-webpki` limitation claim. It returned status `unclear` with confidence
  0.30 and the rationale "Passages were retrieved, but automated semantic support
  or contradiction assessment is unavailable; review the cited passages
  manually." The passages it retrieved contain the quoted limitations text, and
  the same text was fetched directly from
  `https://raw.githubusercontent.com/rustls/webpki/main/README.md`, so the claim
  rests on the primary text rather than on the automated assessment. No other
  source_check call was made, so no other claim carries that validation.
- **Missing evidence.** The OpenSSH certificate format was not assessed (the
  upstream file moved). `cms`, `josekit`, and `tough` were not in the sibling
  measurement, so their transitive cost is unknown. No crate was built, so no
  compile-time or runtime property is asserted beyond the measured trees and the
  version-pinned dependency lists. The claim that an X.509-based verifier would
  need hundreds of lines is an estimate from the RFC's rule list, not a count of
  any existing implementation.

## Sources

- Kept: `.work/signing-format/deps.md`, the measured third-party package counts
  and the C and `cc` build findings, the basis of the cost column.
- Kept: RFC 5280, RFC 8410, RFC 5652, RFC 7515, RFC 7519, RFC 9052, RFC 9360,
  RFC 8392, RFC 9580, RFC 8032, the specifications that state what each
  candidate can carry.
- Kept: `docs.rs` pages and repository READMEs for `der`, `x509-cert`,
  `x509-verify`, `rustls-webpki`, `coset`, `pgp`, and the crates.io metadata for
  `webpki`, the Rust implementation evidence.
- Kept: the crates.io dependency API responses for the pinned crate versions,
  direct dependencies only.
- Kept: minisign's specification page and reference header; the TUF
  specification; the PASETO specification documents.
- Kept (local): `plugin-signing/task.md`, `plugin-signing-research/findings.md`,
  `docs/plugin-contract.md`, `Cargo.toml`, `crates/plugin-runtime/Cargo.toml`,
  `plugin-signing-format/task.md`.
- Rejected: SEO summaries of the same RFCs, non-pinned blog posts, and the
  OpenSSH certificate-protocol file, which is not retrievable at its former URL.

## Next steps

1. `plugin-signing` records the chain and signature encoding in
   `docs/plugin-contract.md` once the operator settles the four questions in
   section 10.
2. The measured dependency cost is now in hand at
   `.work/signing-format/deps.md`; the remaining cost input is the signing side
   for the SDK CLI, where `minisign` at 21 packages and no C build is the
   measured option if the flat-signature scheme is kept.
3. If the operator later wants a second implementation of the format to
   interoperate, section 3 is written to be that specification, and a test vector
   (one root, one link, a 26-byte description) should be committed with it.

## Later decision, 2026-10-14

`plugin-signing` chose X.509 with a project profile instead of the project format
this note recommends. The operator asked which fields had no X.509 home, and the
answer is that none do: the child key and the parent signature are native, the
description is the subject `CN`, and validity is `notBefore` and `notAfter`. The
millisecond expiry that this note treated as a requirement was this effort's own
invention and no consumer needs it, so it was dropped. Two further reasons
decided it: standard issuance tooling (`openssl`, `rcgen`) means the SDK CLI does
not have to issue the chain, and the strict DER decoder makes a short
hand-written path check safe, because non-canonical input is refused before it is
interpreted.

The cost analysis in this note stands and was not the deciding factor:
`x509-cert` adds 13 third-party packages with no C build, and the profile, the
trust set, the local revocation list and the path check remain the project's own
work either way. The recommendation and the rejection list above are historical.
