//! The bundle verifier: the container, the certificate chain and the payload
//! signature.
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

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use x509_cert::Certificate;
use x509_cert::der::{Decode, Reader, SliceReader};

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
        if bytes.len() < HEADER_LENGTH {
            return Err(BundleError::Truncated { len: bytes.len() });
        }
        let magic: [u8; 8] = bytes[0..8].try_into().expect("the header holds the magic");
        if magic != CONTAINER_MAGIC {
            return Err(BundleError::BadMagic { found: magic });
        }
        let container_version = u16::from_be_bytes([bytes[8], bytes[9]]);
        if container_version != CONTAINER_VERSION {
            return Err(BundleError::UnknownContainerVersion {
                found: container_version,
            });
        }
        let header_length = u16::from_be_bytes([bytes[12], bytes[13]]);
        if usize::from(header_length) != HEADER_LENGTH {
            return Err(BundleError::BadHeaderLength { found: header_length });
        }
        let chain_length = u32::from_be_bytes(bytes[16..20].try_into().expect("four bytes"));
        let signature_offset = u64::from_be_bytes(bytes[20..28].try_into().expect("eight bytes"));
        if signature_offset != u64::from(header_length) + u64::from(chain_length) {
            return Err(BundleError::ChainSpanMismatch {
                chain_length,
                signature_offset,
            });
        }
        Ok(BundleHeader {
            container_version,
            signing_suite: u16::from_be_bytes([bytes[10], bytes[11]]),
            header_length,
            reserved: u16::from_be_bytes([bytes[14], bytes[15]]),
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
        if self
            .roots
            .iter()
            .any(|root| root.public_key == public_key)
        {
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
    /// The declared header length is not the fixed one.
    BadHeaderLength { found: u16 },
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
    /// A certificate does not decode.
    MalformedCertificate { link: usize, detail: String },
    /// The anchor link's key id is not a root key.
    UnknownRoot { key_id: KeyId },
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
            BundleError::BadHeaderLength { found } => write!(
                f,
                "the header declares its own length as {found}, and it is {HEADER_LENGTH}"
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
            BundleError::MalformedCertificate { link, detail } => {
                write!(f, "the certificate at link {link} does not decode: {detail}")
            }
            BundleError::UnknownRoot { key_id } => write!(
                f,
                "the chain anchors at {}, which is not a root key",
                hex(key_id)
            ),
            BundleError::MalformedSignature { length } => write!(
                f,
                "the signature is {length} bytes, and an Ed25519 signature is {SIGNATURE_LENGTH}"
            ),
            BundleError::SignatureInvalid => {
                write!(f, "the payload signature does not verify")
            }
            BundleError::MissingCommonName => write!(
                f,
                "the leaf-authorising certificate has no common name"
            ),
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
            | BundleError::MalformedSignature { .. }
            | BundleError::SignatureInvalid
            | BundleError::MissingCommonName => true,
            BundleError::Truncated { .. }
            | BundleError::BadMagic { .. }
            | BundleError::UnknownContainerVersion { .. }
            | BundleError::BadHeaderLength { .. }
            | BundleError::ChainSpanMismatch { .. }
            | BundleError::SpanMismatch { .. }
            | BundleError::MalformedCertificate { .. } => false,
        }
    }
}

/// The hex of a key id, for a message a human reads.
fn hex(id: &KeyId) -> String {
    id.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Verifies a bundle's container, its certificate chain and its payload
/// signature.
///
/// The chain is verified against `trust`, and the payload signature is verified
/// over the domain tag, the header bytes, the chain bytes and the archive bytes,
/// which is to say over every byte of the file except the signature itself.
pub fn verify_bundle<'a>(
    bytes: &'a [u8],
    trust: &TrustStore,
) -> Result<VerifiedBundle<'a>, BundleError> {
    let header = BundleHeader::parse(bytes)?;
    let signature_offset = usize::try_from(header.signature_offset)
        .map_err(|_| too_wide(bytes, &header))?;
    let signature_length = usize::try_from(header.signature_length)
        .map_err(|_| too_wide(bytes, &header))?;
    let zip_length = usize::try_from(header.zip_length).map_err(|_| too_wide(bytes, &header))?;
    let span = signature_offset
        .checked_add(signature_length)
        .and_then(|end| end.checked_add(zip_length));
    if span != Some(bytes.len()) || signature_offset < HEADER_LENGTH {
        return Err(too_wide(bytes, &header));
    }
    let chain = parse_chain(&bytes[HEADER_LENGTH..signature_offset])?;
    let anchor = chain.last().expect("a parsed chain has at least one link");
    let anchor_id = key_id(&link_public_key(anchor, chain.len() - 1)?);
    if !trust.trusts(&anchor_id) {
        return Err(BundleError::UnknownRoot { key_id: anchor_id });
    }
    let signature_bytes = &bytes[signature_offset..signature_offset + signature_length];
    if signature_bytes.len() != SIGNATURE_LENGTH {
        return Err(BundleError::MalformedSignature {
            length: signature_bytes.len(),
        });
    }
    let signature = Signature::from_slice(signature_bytes).map_err(|_| {
        BundleError::MalformedSignature {
            length: signature_bytes.len(),
        }
    })?;
    let leaf = &chain[0];
    let leaf_key = verifying_key(&link_public_key(leaf, 0)?, 0)?;
    let message = signed_message(
        &bytes[..HEADER_LENGTH],
        &bytes[HEADER_LENGTH..signature_offset],
        &bytes[signature_offset + signature_length..],
    );
    leaf_key
        .verify_strict(&message, &signature)
        .map_err(|_| BundleError::SignatureInvalid)?;
    Ok(VerifiedBundle {
        header,
        archive: &bytes[signature_offset + signature_length..],
        leaf_common_name: leaf_common_name(leaf)?,
    })
}

/// The span the header declares does not match the file.
///
/// A field that does not fit a `usize` on this host, a sum that overflows, a
/// signature that would begin inside the header, or a total that is not the
/// file's length all arrive here: each means some part of the file is outside
/// the range the signature covers.
fn too_wide(bytes: &[u8], header: &BundleHeader) -> BundleError {
    BundleError::SpanMismatch {
        file_length: bytes.len(),
        signature_offset: header.signature_offset,
        signature_length: header.signature_length,
        zip_length: header.zip_length,
    }
}

/// The message the payload signature covers.
fn signed_message(header: &[u8], chain: &[u8], archive: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(DOMAIN_TAG.len() + header.len() + chain.len() + archive.len());
    message.extend_from_slice(DOMAIN_TAG);
    message.extend_from_slice(header);
    message.extend_from_slice(chain);
    message.extend_from_slice(archive);
    message
}

/// Reads the chain region into its links, leaf first.
fn parse_chain(region: &[u8]) -> Result<Vec<Certificate>, BundleError> {
    let mut reader = SliceReader::new(region).map_err(|err| BundleError::MalformedCertificate {
        link: 0,
        detail: err.to_string(),
    })?;
    let mut links = Vec::new();
    while !reader.is_finished() {
        let link = links.len();
        let certificate =
            Certificate::decode(&mut reader).map_err(|err| BundleError::MalformedCertificate {
                link,
                detail: err.to_string(),
            })?;
        links.push(certificate);
    }
    if links.is_empty() {
        return Err(BundleError::EmptyChain);
    }
    Ok(links)
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

/// The leaf-authorising certificate's common name.
fn leaf_common_name(certificate: &Certificate) -> Result<String, BundleError> {
    match certificate.tbs_certificate().subject().common_name() {
        Ok(Some(name)) => Ok(name.value().into_owned()),
        Ok(None) => Err(BundleError::MissingCommonName),
        Err(err) => Err(BundleError::MalformedCertificate {
            link: 0,
            detail: err.to_string(),
        }),
    }
}
