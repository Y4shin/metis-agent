//! The bundle's two halves: the verifier, which turns a file into a borrowed
//! view of it or a typed refusal, and the extractor, which turns that view into
//! a staging directory this host's load reads.
//!
//! A bundle is a file, and verifying one turns the file into either a borrowed
//! view of it or a typed refusal. Nothing is extracted and nothing is copied:
//! the archive a caller reads is the caller's own bytes.
//!
//! The container's layout, the domain tag, the key id rule, the revocation rule
//! and every refusal are the contract's, in `docs/plugin-contract.md`. The job
//! here is to enforce them without panicking on a hostile file, so the container
//! is read field by field and the certificate chain is read by a strict DER
//! decoder rather than by this crate.
//!
//! The daemon owns the trust policy. This module never reads a configuration
//! file: it takes a [`TrustStore`] as a plain value, and the compiled-in default
//! trusts nothing.

use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use x509_cert::Certificate;
use x509_cert::der::oid::AssociatedOid;
use x509_cert::der::{Decode, Encode, ErrorKind, Reader, SliceReader};
use x509_cert::ext::pkix::BasicConstraints;
use x509_cert::time::Time;

mod extract;

// The extractor is reached as `bundle::extract`, next to the verifier it takes
// its input from, so a caller names one module for both halves of the bundle
// path.
pub use extract::{MAX_ENTRIES, MAX_ENTRY_SIZE, MAX_UNCOMPRESSED_SIZE, STAGING_PREFIX};
pub use extract::{StageError, StagedBundle, extract};

/// The container's magic, at offset 0.
pub const CONTAINER_MAGIC: [u8; 8] = *b"METISBND";

/// The container format version this build reads. A bundle carries the version
/// at a fixed offset and it must match this exactly.
pub const CONTAINER_VERSION: u16 = 1;

/// The signing suite this build reads: X.509 over Ed25519.
pub const SIGNING_SUITE: u16 = 1;

/// The fixed header length, and the offset at which the chain region begins.
pub const HEADER_LENGTH: usize = 40;

/// The most links a chain may hold.
pub const MAX_CHAIN_LINKS: usize = 8;

/// The length of a key id.
pub const KEY_ID_LENGTH: usize = 16;

/// The domain tag the payload signature covers, ahead of the header, the chain
/// and the archive. It is fixed and carries no container version, so a version
/// bump does not change what a signature covers.
pub const DOMAIN_TAG: &[u8] = b"METIS-PLUGIN-BUNDLE-PAYLOAD";

/// An Ed25519 signature's length.
const SIGNATURE_LENGTH: usize = 64;

/// An Ed25519 public key's length.
const PUBLIC_KEY_LENGTH: usize = 32;

/// The container's fixed header, big-endian throughout.
///
/// The header is fixed width because the signature's location is computed from
/// it: the chain region follows the header, the signature follows the chain, and
/// the archive follows the signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundleHeader {
    /// The container format version, which must equal [`CONTAINER_VERSION`].
    pub container_version: u16,
    /// The signing suite, which must equal [`SIGNING_SUITE`].
    pub signing_suite: u16,
    /// The header's own length, which must equal [`HEADER_LENGTH`].
    pub header_length: u16,
    /// A reserved field, which must be zero.
    pub reserved: u16,
    /// The chain region's length in bytes.
    pub chain_length: u32,
    /// Where the signature starts: the header's length plus the chain's.
    pub signature_offset: u64,
    /// The signature's length in bytes.
    pub signature_length: u32,
    /// The archive's length in bytes.
    pub zip_length: u64,
}

impl BundleHeader {
    /// Reads the fixed header from the first forty bytes.
    ///
    /// The magic and the container version are read first, and the rest of the
    /// header is shaped by the version, which is why a version this build does
    /// not read is refused before any other field is interpreted.
    pub fn parse(bytes: &[u8]) -> Result<BundleHeader, BundleError> {
        // The magic first, so a file that is not a bundle answers to that and not
        // to whatever the rest of its first bytes happen to say.
        if bytes.len() < CONTAINER_MAGIC.len() {
            return Err(BundleError::Truncated { len: bytes.len() });
        }
        let magic: [u8; 8] = bytes[0..8].try_into().expect("the header holds the magic");
        if magic != CONTAINER_MAGIC {
            return Err(BundleError::BadMagic { found: magic });
        }
        if bytes.len() < HEADER_LENGTH {
            return Err(BundleError::Truncated { len: bytes.len() });
        }
        let container_version = u16::from_be_bytes([bytes[8], bytes[9]]);
        if container_version != CONTAINER_VERSION {
            return Err(BundleError::UnknownContainerVersion {
                found: container_version,
            });
        }
        let signing_suite = u16::from_be_bytes([bytes[10], bytes[11]]);
        if signing_suite != SIGNING_SUITE {
            return Err(BundleError::UnknownSigningSuite {
                found: signing_suite,
            });
        }
        let header_length = u16::from_be_bytes([bytes[12], bytes[13]]);
        if usize::from(header_length) != HEADER_LENGTH {
            return Err(BundleError::BadHeaderLength {
                found: header_length,
            });
        }
        let reserved = u16::from_be_bytes([bytes[14], bytes[15]]);
        if reserved != 0 {
            return Err(BundleError::ReservedNotZero { found: reserved });
        }
        let chain_length = u32::from_be_bytes(bytes[16..20].try_into().expect("four bytes"));
        let signature_offset = u64::from_be_bytes(bytes[20..28].try_into().expect("eight bytes"));
        // The chain region runs from the header's end to the signature, so an
        // offset before the header's end would place chain bytes before it.
        if signature_offset < u64::from(header_length) {
            return Err(BundleError::BytesBeforeHeader {
                signature_offset,
                header_length,
            });
        }
        if signature_offset != u64::from(header_length) + u64::from(chain_length) {
            return Err(BundleError::ChainSpanMismatch {
                chain_length,
                signature_offset,
            });
        }
        Ok(BundleHeader {
            container_version,
            signing_suite,
            header_length,
            reserved,
            chain_length,
            signature_offset,
            signature_length: u32::from_be_bytes(bytes[28..32].try_into().expect("four bytes")),
            zip_length: u64::from_be_bytes(bytes[32..40].try_into().expect("eight bytes")),
        })
    }
}

/// A key id: the first sixteen bytes of the SHA-256 of a public key.
pub type KeyId = [u8; KEY_ID_LENGTH];

/// The key id of an Ed25519 public key.
///
/// The digest is taken over the raw 32-byte key, never over an SPKI wrapper, and
/// it is this function rather than a carried identifier that decides what a key
/// is called everywhere else in this module.
pub fn key_id(public_key: &[u8; PUBLIC_KEY_LENGTH]) -> KeyId {
    let digest = Sha256::digest(public_key);
    let mut id = [0u8; KEY_ID_LENGTH];
    id.copy_from_slice(&digest[..KEY_ID_LENGTH]);
    id
}

/// A root key the daemon trusts, registered under the id computed from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootKey {
    id: KeyId,
    public_key: [u8; PUBLIC_KEY_LENGTH],
}

impl RootKey {
    /// The key id, computed from the public key when the root was registered.
    pub fn id(&self) -> KeyId {
        self.id
    }

    /// The raw Ed25519 public key.
    pub fn public_key(&self) -> [u8; PUBLIC_KEY_LENGTH] {
        self.public_key
    }
}

/// The trust policy, as a plain value the daemon constructs.
///
/// It holds the root keys a chain may anchor at and the keys that are revoked.
/// A root is registered by its public key, so its id is recomputed here rather
/// than carried by the caller, and a revoked key is named by that same id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustStore {
    roots: Vec<RootKey>,
    revoked: Vec<KeyId>,
}

impl TrustStore {
    /// The compiled-in default the daemon starts from: it trusts nothing.
    pub fn empty() -> TrustStore {
        TrustStore::default()
    }

    /// The same store with one more root.
    #[must_use]
    pub fn with_root(mut self, public_key: [u8; PUBLIC_KEY_LENGTH]) -> TrustStore {
        self.add_root(public_key);
        self
    }

    /// Registers one root key. A key already registered is not registered twice.
    pub fn add_root(&mut self, public_key: [u8; PUBLIC_KEY_LENGTH]) {
        if self.roots.iter().any(|root| root.public_key == public_key) {
            return;
        }
        self.roots.push(RootKey {
            id: key_id(&public_key),
            public_key,
        });
    }

    /// Revokes one key by the id computed from it.
    pub fn revoke(&mut self, id: KeyId) {
        if !self.revoked.contains(&id) {
            self.revoked.push(id);
        }
    }

    /// The registered roots.
    pub fn roots(&self) -> &[RootKey] {
        &self.roots
    }

    /// The revoked key ids.
    pub fn revoked(&self) -> &[KeyId] {
        &self.revoked
    }

    /// Whether one key id is a registered root.
    fn trusts(&self, id: &KeyId) -> bool {
        self.roots.iter().any(|root| root.id == *id)
    }

    /// Whether one key id is revoked.
    fn revokes(&self, id: &KeyId) -> bool {
        self.revoked.contains(id)
    }
}

/// A borrowed view of an accepted bundle.
#[derive(Debug)]
pub struct VerifiedBundle<'a> {
    header: BundleHeader,
    archive: &'a [u8],
    leaf_common_name: String,
}

impl<'a> VerifiedBundle<'a> {
    /// The parsed container header.
    pub fn header(&self) -> &BundleHeader {
        &self.header
    }

    /// The archive bytes, borrowed from the file that was verified.
    pub fn archive(&self) -> &'a [u8] {
        self.archive
    }

    /// The leaf-authorising certificate's common name, which the manifest check
    /// compares against the plugin's name.
    pub fn leaf_common_name(&self) -> &str {
        &self.leaf_common_name
    }
}

/// What refusing a bundle can say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleError {
    /// The file is shorter than the fixed header.
    Truncated { len: usize },
    /// The magic at offset 0 is not the container's.
    BadMagic { found: [u8; 8] },
    /// The container format version is not the one this build reads.
    UnknownContainerVersion { found: u16 },
    /// The signing suite is not the one this build reads.
    UnknownSigningSuite { found: u16 },
    /// The declared header length is not the fixed one.
    BadHeaderLength { found: u16 },
    /// A reserved field is not zero.
    ReservedNotZero { found: u16 },
    /// The signature offset lies before the header ends, so the chain region
    /// would begin inside the header.
    BytesBeforeHeader {
        signature_offset: u64,
        header_length: u16,
    },
    /// The signature offset is not the header's length plus the chain's.
    ChainSpanMismatch {
        chain_length: u32,
        signature_offset: u64,
    },
    /// The signature and archive spans do not account for the whole file.
    SpanMismatch {
        file_length: usize,
        signature_offset: u64,
        signature_length: u32,
        zip_length: u64,
    },
    /// The chain holds no link.
    EmptyChain,
    /// The chain holds more links than the profile allows.
    ChainTooLong { links: usize },
    /// Bytes after the last whole certificate do not form one.
    TrailingChainBytes { bytes: usize },
    /// A certificate is not canonically encoded: its DER has more octets than
    /// the value needs, or a field is written in a form DER does not use.
    NonCanonicalDer { link: usize, detail: String },
    /// A certificate does not decode.
    MalformedCertificate { link: usize, detail: String },
    /// The anchor link's key id is not a root key.
    UnknownRoot { key_id: KeyId },
    /// A link's key id is on the revocation list.
    RevokedKey { key_id: KeyId },
    /// A link's validity window has passed.
    Expired { link: usize, not_after: i64 },
    /// A link's validity window has not started.
    NotYetValid { link: usize, not_before: i64 },
    /// A link that authorises another key carries no `CA:TRUE` constraint.
    CaConstraintMissing { link: usize },
    /// The leaf-authorising certificate carries the CA constraint.
    LeafIsCa,
    /// A link carries a critical extension this build does not understand.
    UnknownCriticalExtension { link: usize, oid: String },
    /// A child's issuer is not its parent's subject.
    ChainBroken { child: usize },
    /// A child's own signature does not verify under its parent's key.
    ChainSignatureInvalid { child: usize },
    /// The signature's length is not an Ed25519 signature's.
    MalformedSignature { length: usize },
    /// The payload signature does not verify.
    SignatureInvalid,
    /// The leaf-authorising certificate has no common name.
    MissingCommonName,
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleError::Truncated { len } => write!(
                f,
                "the file is {len} bytes, shorter than the {HEADER_LENGTH}-byte header"
            ),
            BundleError::BadMagic { found } => write!(
                f,
                "the file does not begin with the container magic: {found:02x?}"
            ),
            BundleError::UnknownContainerVersion { found } => write!(
                f,
                "the container version is {found}, and this build reads {CONTAINER_VERSION}"
            ),
            BundleError::UnknownSigningSuite { found } => write!(
                f,
                "the signing suite is {found}, and this build reads {SIGNING_SUITE}"
            ),
            BundleError::BadHeaderLength { found } => write!(
                f,
                "the header declares its own length as {found}, and it is {HEADER_LENGTH}"
            ),
            BundleError::ReservedNotZero { found } => {
                write!(f, "a reserved header field is {found}, and it must be zero")
            }
            BundleError::BytesBeforeHeader {
                signature_offset,
                header_length,
            } => write!(
                f,
                "the signature offset {signature_offset} is before the header's end at \
                 {header_length}, so the chain region would begin inside the header"
            ),
            BundleError::ChainSpanMismatch {
                chain_length,
                signature_offset,
            } => write!(
                f,
                "the signature offset {signature_offset} is not the header length plus the \
                 chain length {chain_length}"
            ),
            BundleError::SpanMismatch {
                file_length,
                signature_offset,
                signature_length,
                zip_length,
            } => write!(
                f,
                "the file is {file_length} bytes, and the signature at {signature_offset} plus \
                 its {signature_length} bytes and the archive's {zip_length} bytes do not \
                 account for it"
            ),
            BundleError::EmptyChain => write!(f, "the chain region holds no certificate"),
            BundleError::ChainTooLong { links } => write!(
                f,
                "the chain holds {links} links, and it may hold {MAX_CHAIN_LINKS}"
            ),
            BundleError::TrailingChainBytes { bytes } => write!(
                f,
                "the chain region carries {bytes} bytes after the last whole certificate"
            ),
            BundleError::NonCanonicalDer { link, detail } => write!(
                f,
                "the certificate at link {link} is not canonically encoded: {detail}"
            ),
            BundleError::MalformedCertificate { link, detail } => {
                write!(
                    f,
                    "the certificate at link {link} does not decode: {detail}"
                )
            }
            BundleError::UnknownRoot { key_id } => write!(
                f,
                "the chain anchors at {}, which is not a root key",
                hex(key_id)
            ),
            BundleError::RevokedKey { key_id } => {
                write!(f, "the key {} is revoked", hex(key_id))
            }
            BundleError::Expired { link, not_after } => {
                write!(f, "the certificate at link {link} expired at {not_after}")
            }
            BundleError::NotYetValid { link, not_before } => write!(
                f,
                "the certificate at link {link} is not valid until {not_before}"
            ),
            BundleError::CaConstraintMissing { link } => write!(
                f,
                "the certificate at link {link} authorises another key without CA:TRUE"
            ),
            BundleError::LeafIsCa => write!(
                f,
                "the leaf-authorising certificate authorises other keys, and must be an end entity"
            ),
            BundleError::UnknownCriticalExtension { link, oid } => write!(
                f,
                "the certificate at link {link} carries the critical extension {oid}"
            ),
            BundleError::ChainBroken { child } => write!(
                f,
                "the certificate at link {child} names an issuer that is not its parent's subject"
            ),
            BundleError::ChainSignatureInvalid { child } => write!(
                f,
                "the certificate at link {child} is not signed by its parent's key"
            ),
            BundleError::MalformedSignature { length } => write!(
                f,
                "the signature is {length} bytes, and an Ed25519 signature is {SIGNATURE_LENGTH}"
            ),
            BundleError::SignatureInvalid => {
                write!(f, "the payload signature does not verify")
            }
            BundleError::MissingCommonName => {
                write!(f, "the leaf-authorising certificate has no common name")
            }
        }
    }
}

impl std::error::Error for BundleError {}

impl BundleError {
    /// Whether this refusal means "this file claims a signature this daemon does
    /// not trust".
    ///
    /// A trust failure stops a daemon's start-up, because a file that claims an
    /// untrusted signature is evidence that something is wrong. The container
    /// outcomes are files this build cannot read without evidence of tampering,
    /// so they degrade that one bundle instead.
    pub fn is_trust_failure(&self) -> bool {
        match self {
            BundleError::EmptyChain
            | BundleError::UnknownRoot { .. }
            | BundleError::RevokedKey { .. }
            | BundleError::Expired { .. }
            | BundleError::NotYetValid { .. }
            | BundleError::CaConstraintMissing { .. }
            | BundleError::LeafIsCa
            | BundleError::UnknownCriticalExtension { .. }
            | BundleError::ChainBroken { .. }
            | BundleError::ChainSignatureInvalid { .. }
            | BundleError::MalformedSignature { .. }
            | BundleError::SignatureInvalid
            | BundleError::MissingCommonName => true,
            BundleError::Truncated { .. }
            | BundleError::BadMagic { .. }
            | BundleError::UnknownContainerVersion { .. }
            | BundleError::UnknownSigningSuite { .. }
            | BundleError::BadHeaderLength { .. }
            | BundleError::ReservedNotZero { .. }
            | BundleError::BytesBeforeHeader { .. }
            | BundleError::ChainSpanMismatch { .. }
            | BundleError::SpanMismatch { .. }
            | BundleError::ChainTooLong { .. }
            | BundleError::TrailingChainBytes { .. }
            | BundleError::NonCanonicalDer { .. }
            | BundleError::MalformedCertificate { .. } => false,
        }
    }
}

/// The hex of a key id, for a message a human reads.
fn hex(id: &KeyId) -> String {
    id.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// How far a certificate's validity window may be missed before it is refused.
///
/// A clock is not exact and a producer's signing host is not this host, so a
/// minute is tolerated at both ends of the window.
const CLOCK_SKEW_SECONDS: i64 = 60;

/// Verifies a bundle's container, its certificate chain and its payload
/// signature.
///
/// The checks run in one order, and each answers for itself rather than for a
/// later consequence: the container, then the chain region, then the anchor, the
/// revocation list, the validity windows, the chain profile, the links, and
/// finally the payload signature over the domain tag, the header bytes, the
/// chain bytes and the archive bytes, which is to say over every byte of the
/// file except the signature itself.
pub fn verify_bundle<'a>(
    bytes: &'a [u8],
    trust: &TrustStore,
) -> Result<VerifiedBundle<'a>, BundleError> {
    let header = BundleHeader::parse(bytes)?;
    let offsets = RegionOffsets::of(bytes, &header)?;
    let chain = parse_chain(offsets.chain(bytes))?;
    let keys = chain_keys(&chain)?;
    check_anchor(&keys, trust)?;
    check_revocation(&keys, trust)?;
    check_validity(&chain, clock_seconds())?;
    let leaf_common_name = check_profile(&chain)?;
    check_links(&chain, &keys)?;
    // The chain holds at least one link, or `EmptyChain` was refused above.
    check_payload_signature(&keys[0], &offsets, bytes)?;
    Ok(VerifiedBundle {
        header,
        // The span check above proved the archive ends exactly at the file's
        // end, so the range is inside the file and this cannot fail.
        archive: bytes
            .get(offsets.archive())
            .expect("the span covers the archive"),
        leaf_common_name,
    })
}

/// Where each region of the file is, once the header's span checks have passed.
struct RegionOffsets {
    signature_offset: usize,
    signature_length: usize,
    file_length: usize,
}

impl RegionOffsets {
    /// Checks the declared span against the file's length and returns the
    /// offsets.
    ///
    /// A field that does not fit a `usize` on this host, a sum that overflows,
    /// and a total that is not the file's length all arrive here: each means
    /// some part of the file would fall outside the range the signature covers.
    fn of(bytes: &[u8], header: &BundleHeader) -> Result<RegionOffsets, BundleError> {
        let mismatch = || BundleError::SpanMismatch {
            file_length: bytes.len(),
            signature_offset: header.signature_offset,
            signature_length: header.signature_length,
            zip_length: header.zip_length,
        };
        let signature_offset = usize::try_from(header.signature_offset).map_err(|_| mismatch())?;
        let signature_length = usize::try_from(header.signature_length).map_err(|_| mismatch())?;
        let zip_length = usize::try_from(header.zip_length).map_err(|_| mismatch())?;
        let span = signature_offset
            .checked_add(signature_length)
            .and_then(|end| end.checked_add(zip_length));
        if span != Some(bytes.len()) {
            return Err(mismatch());
        }
        Ok(RegionOffsets {
            signature_offset,
            signature_length,
            file_length: bytes.len(),
        })
    }

    /// The chain region: after the header, up to the signature.
    fn chain<'a>(&self, bytes: &'a [u8]) -> &'a [u8] {
        &bytes[HEADER_LENGTH..self.signature_offset]
    }

    /// The signature region.
    fn signature<'a>(&self, bytes: &'a [u8]) -> &'a [u8] {
        &bytes[self.signature_offset..self.signature_offset + self.signature_length]
    }

    /// The archive region, at the end of the file.
    fn archive(&self) -> std::ops::Range<usize> {
        self.signature_offset + self.signature_length..self.file_length
    }
}

/// The message the payload signature covers.
fn signed_message(header: &[u8], chain: &[u8], archive: &[u8]) -> Vec<u8> {
    let mut message =
        Vec::with_capacity(DOMAIN_TAG.len() + header.len() + chain.len() + archive.len());
    message.extend_from_slice(DOMAIN_TAG);
    message.extend_from_slice(header);
    message.extend_from_slice(chain);
    message.extend_from_slice(archive);
    message
}

/// Reads the chain region into its links, leaf first.
///
/// The region is a concatenation of whole DER certificates. A link that does not
/// decode is a malformed certificate when it is the first one, and the bytes
/// left over after a whole link when it follows one, which is how the region's
/// trailing-bytes rule is told apart from a malformed region.
fn parse_chain(region: &[u8]) -> Result<Vec<Certificate>, BundleError> {
    let mut reader = SliceReader::new(region).map_err(|err| BundleError::MalformedCertificate {
        link: 0,
        detail: err.to_string(),
    })?;
    let mut links: Vec<Certificate> = Vec::new();
    while !reader.is_finished() {
        let link = links.len();
        let start = usize::try_from(reader.position()).unwrap_or(0);
        match Certificate::decode(&mut reader) {
            Ok(certificate) => links.push(certificate),
            Err(err) => return Err(chain_decode_error(region, start, link, &err)),
        }
        if links.len() > MAX_CHAIN_LINKS {
            return Err(BundleError::ChainTooLong { links: links.len() });
        }
    }
    if links.is_empty() {
        return Err(BundleError::EmptyChain);
    }
    Ok(links)
}

/// What a chain region that did not decode says.
///
/// The decoder is strict, so an encoding fault is refused before the value is
/// interpreted: a boolean that is true in BER but not in DER, or a length that
/// is not the minimal one for its value. That is a canonicality refusal rather
/// than a malformed certificate. Anything else is bytes that do not form a
/// certificate.
fn chain_decode_error(region: &[u8], start: usize, link: usize, err: &der::Error) -> BundleError {
    match err.kind() {
        ErrorKind::Noncanonical { .. } | ErrorKind::Overlength | ErrorKind::Length { .. } => {
            BundleError::NonCanonicalDer {
                link,
                detail: err.to_string(),
            }
        }
        _ if link == 0 => BundleError::MalformedCertificate {
            link,
            detail: err.to_string(),
        },
        _ => BundleError::TrailingChainBytes {
            bytes: region.len().saturating_sub(start),
        },
    }
}

/// One link's key, with the id computed from it rather than carried by it.
struct LinkKey {
    id: KeyId,
    public_key: [u8; PUBLIC_KEY_LENGTH],
}

/// The key of every link, leaf first.
fn chain_keys(chain: &[Certificate]) -> Result<Vec<LinkKey>, BundleError> {
    chain
        .iter()
        .enumerate()
        .map(|(link, certificate)| {
            link_public_key(certificate, link).map(|public_key| LinkKey {
                id: key_id(&public_key),
                public_key,
            })
        })
        .collect()
}

/// The anchor is the last link, and its key id has to name a root key.
///
/// The anchor is trusted by its key rather than by who signed it: a root key is
/// a key the daemon holds, and a certificate that carries that key says nothing
/// an attacker could not also say. What the anchor cannot do without the root's
/// private key is sign the link below it, which is what the links are checked
/// against.
fn check_anchor(keys: &[LinkKey], trust: &TrustStore) -> Result<(), BundleError> {
    let anchor = keys.last().expect("a parsed chain has at least one link");
    if trust.trusts(&anchor.id) {
        Ok(())
    } else {
        Err(BundleError::UnknownRoot { key_id: anchor.id })
    }
}

/// No key in the chain may be revoked, the anchor included.
fn check_revocation(keys: &[LinkKey], trust: &TrustStore) -> Result<(), BundleError> {
    for key in keys {
        if trust.revokes(&key.id) {
            return Err(BundleError::RevokedKey { key_id: key.id });
        }
    }
    Ok(())
}

/// Every link's own validity window, with the documented skew.
fn check_validity(chain: &[Certificate], now: i64) -> Result<(), BundleError> {
    for (link, certificate) in chain.iter().enumerate() {
        let validity = certificate.tbs_certificate().validity();
        let not_before = unix_seconds(validity.not_before);
        let not_after = unix_seconds(validity.not_after);
        if now < not_before.saturating_sub(CLOCK_SKEW_SECONDS) {
            return Err(BundleError::NotYetValid { link, not_before });
        }
        if now > not_after.saturating_add(CLOCK_SKEW_SECONDS) {
            return Err(BundleError::Expired { link, not_after });
        }
    }
    Ok(())
}

/// The profile this project owns: the CA constraint, the end entity leaf, the
/// extensions this build understands, and the leaf's common name.
fn check_profile(chain: &[Certificate]) -> Result<String, BundleError> {
    for (link, certificate) in chain.iter().enumerate() {
        check_extensions(certificate, link)?;
        let constraints = basic_constraints(certificate, link)?;
        if link == 0 {
            if constraints == Some(true) {
                return Err(BundleError::LeafIsCa);
            }
        } else if constraints != Some(true) {
            return Err(BundleError::CaConstraintMissing { link });
        }
    }
    let leaf = chain.first().expect("a parsed chain has at least one link");
    let name = leaf
        .tbs_certificate()
        .subject()
        .common_name()
        .map_err(|err| BundleError::MalformedCertificate {
            link: 0,
            detail: err.to_string(),
        })?;
    // The name is the plugin's name, which the manifest check compares. An
    // absent or empty one is not a name.
    match name.map(|name| name.value().into_owned()) {
        Some(name) if !name.is_empty() => Ok(name),
        _ => Err(BundleError::MissingCommonName),
    }
}

/// Every critical extension has to be one this build understands.
///
/// `basicConstraints` is the only extension the profile interprets. Another
/// critical extension refuses the chain, and another non-critical one is
/// ignored, which is what lets a producer carry a key identifier without asking
/// this build to trust it.
fn check_extensions(certificate: &Certificate, link: usize) -> Result<(), BundleError> {
    let extensions = certificate.tbs_certificate().extensions();
    for extension in extensions.into_iter().flatten() {
        if extension.critical && extension.extn_id != BasicConstraints::OID {
            return Err(BundleError::UnknownCriticalExtension {
                link,
                oid: extension.extn_id.to_string(),
            });
        }
    }
    Ok(())
}

/// The `CA:TRUE` constraint a certificate carries, if it carries one.
fn basic_constraints(certificate: &Certificate, link: usize) -> Result<Option<bool>, BundleError> {
    match certificate
        .tbs_certificate()
        .get_extension::<BasicConstraints>()
    {
        Ok(Some((_critical, constraints))) => Ok(Some(constraints.ca)),
        Ok(None) => Ok(None),
        Err(err) => Err(BundleError::MalformedCertificate {
            link,
            detail: err.to_string(),
        }),
    }
}

/// Each child names its parent as its issuer, and each parent signed its child.
///
/// The signed bytes are the child's re-encoded certificate body. That is sound
/// only because the chain was parsed by the strict decoder, which refused any
/// body whose encoding is not the canonical one.
fn check_links(chain: &[Certificate], keys: &[LinkKey]) -> Result<(), BundleError> {
    for child in 0..chain.len().saturating_sub(1) {
        let parent = &chain[child + 1];
        if chain[child].tbs_certificate().issuer() != parent.tbs_certificate().subject() {
            return Err(BundleError::ChainBroken { child });
        }
        let key = verifying_key(&keys[child + 1].public_key, child + 1)?;
        let body = chain[child].tbs_certificate().to_der().map_err(|err| {
            BundleError::MalformedCertificate {
                link: child,
                detail: err.to_string(),
            }
        })?;
        let signature = chain[child].signature().as_bytes().ok_or_else(|| {
            BundleError::MalformedCertificate {
                link: child,
                detail: "the link's signature is not octet aligned".to_string(),
            }
        })?;
        let signature =
            Signature::from_slice(signature).map_err(|err| BundleError::MalformedCertificate {
                link: child,
                detail: err.to_string(),
            })?;
        key.verify_strict(&body, &signature)
            .map_err(|_| BundleError::ChainSignatureInvalid { child })?;
    }
    Ok(())
}

/// The payload signature, by the leaf-authorising certificate's key.
fn check_payload_signature(
    leaf: &LinkKey,
    offsets: &RegionOffsets,
    bytes: &[u8],
) -> Result<(), BundleError> {
    let signature_bytes = offsets.signature(bytes);
    if signature_bytes.len() != SIGNATURE_LENGTH {
        return Err(BundleError::MalformedSignature {
            length: signature_bytes.len(),
        });
    }
    let signature =
        Signature::from_slice(signature_bytes).map_err(|_| BundleError::MalformedSignature {
            length: signature_bytes.len(),
        })?;
    let key = verifying_key(&leaf.public_key, 0)?;
    let message = signed_message(
        &bytes[..HEADER_LENGTH],
        offsets.chain(bytes),
        &bytes[offsets.archive()],
    );
    key.verify_strict(&message, &signature)
        .map_err(|_| BundleError::SignatureInvalid)
}

/// Seconds since the unix epoch, as a certificate's validity is expressed.
///
/// A clock before the epoch reads as zero rather than panicking: refusing every
/// bundle on such a host is the honest answer, and it is what an unset clock
/// means.
fn clock_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

/// One certificate time, in seconds since the unix epoch.
fn unix_seconds(time: Time) -> i64 {
    time.to_unix_duration().as_secs() as i64
}

/// A link's raw Ed25519 public key.
fn link_public_key(
    certificate: &Certificate,
    link: usize,
) -> Result<[u8; PUBLIC_KEY_LENGTH], BundleError> {
    // A key is octet aligned, so a `BIT STRING` with unused bits is not a key.
    let raw = certificate
        .tbs_certificate()
        .subject_public_key_info()
        .subject_public_key
        .as_bytes()
        .ok_or_else(|| BundleError::MalformedCertificate {
            link,
            detail: "the public key is not octet aligned".to_string(),
        })?;
    <[u8; PUBLIC_KEY_LENGTH]>::try_from(raw).map_err(|_| BundleError::MalformedCertificate {
        link,
        detail: format!(
            "the public key is {} bytes, not {PUBLIC_KEY_LENGTH}",
            raw.len()
        ),
    })
}

/// A link's public key as a verifying key.
fn verifying_key(
    public_key: &[u8; PUBLIC_KEY_LENGTH],
    link: usize,
) -> Result<VerifyingKey, BundleError> {
    VerifyingKey::from_bytes(public_key).map_err(|err| BundleError::MalformedCertificate {
        link,
        detail: err.to_string(),
    })
}
