//! A test-only signer: it issues a small certificate chain and signs a bundle.
//!
//! This file is a second implementation of the container's rules, written from
//! `docs/plugin-contract.md` rather than from the verifier's constants. That is
//! deliberate: a committed vector built here proves the contract's offsets, its
//! domain tag and its chain rules independently of the code that reads them. If
//! the two ever disagree about a byte, the vector test fails.
//!
//! Nothing here is compiled into a shipped build, and no key here is in any
//! compiled-in root set. The keys come from fixed seeds, so a committed vector
//! is reproducible, and a test installs the anchor as a root explicitly.

use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey};
use x509_cert::builder::profile::BuilderProfile;
use x509_cert::builder::{Builder, CertificateBuilder};
use x509_cert::certificate::{Certificate, TbsCertificate};
use x509_cert::der::asn1::OctetString;
use x509_cert::der::oid::AssociatedOid;
use x509_cert::der::{DateTime, Encode};
use x509_cert::ext::pkix::BasicConstraints;
use x509_cert::ext::Extension;
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::spki::{SubjectPublicKeyInfo, SubjectPublicKeyInfoRef};
use x509_cert::time::{Time, Validity};

/// The container's magic, at offset 0.
pub const MAGIC: [u8; 8] = *b"METISBND";

/// The container format version this build writes.
pub const CONTAINER_VERSION: u16 = 1;

/// The signing suite: X.509 over Ed25519.
pub const SIGNING_SUITE: u16 = 1;

/// The fixed header length.
pub const HEADER_LENGTH: u16 = 40;

/// The domain tag the payload signature covers, ahead of the header, the chain
/// and the archive. It carries no container version.
pub const DOMAIN_TAG: &[u8] = b"METIS-PLUGIN-BUNDLE-PAYLOAD";

/// The common name of the leaf-authorising certificate, which the manifest must
/// name.
pub const LEAF_COMMON_NAME: &str = "dev.metis.test-plugin";

/// An empty ZIP archive: the end-of-central-directory record alone. The
/// verifier treats the archive as opaque bytes, so a vector only needs bytes
/// that a later reader can open.
pub const EMPTY_ZIP: &[u8] = b"PK\x05\x06\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";

/// The validity window every standard link carries: inside it until 2040, so a
/// committed vector keeps verifying.
pub const NOT_BEFORE: i64 = 1_704_067_200; // 2024-01-01T00:00:00Z
pub const NOT_AFTER: i64 = 2_208_988_800; // 2040-01-01T00:00:00Z

/// The key of one link, from a fixed seed so every run is reproducible.
pub fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

/// The raw 32-byte public key of a signing key.
pub fn public_key(key: &SigningKey) -> [u8; 32] {
    key.verifying_key().to_bytes()
}

/// Seconds since the unix epoch, as a certificate's validity is expressed.
pub fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("this test host's clock is after the unix epoch")
        .as_secs() as i64
}

/// How one link's certificate is built.
#[derive(Clone)]
pub struct LinkSpec {
    /// The subject's common name.
    pub common_name: String,
    /// The issuer's common name, which names the link above this one.
    pub issuer_common_name: String,
    /// The `basicConstraints` this link carries: `Some(true)` for a certificate
    /// that authorises another key, `Some(false)` for an end entity, `None` for
    /// a certificate with no `basicConstraints` at all.
    pub ca: Option<bool>,
    /// Extensions added after `basicConstraints`, such as a carried key
    /// identifier or an extension the verifier does not understand.
    pub extensions: Vec<Extension>,
    /// The validity window, in unix seconds.
    pub validity: (i64, i64),
    /// The serial number.
    pub serial: u32,
}

/// The common name of the link at `index`, where the last link is the anchor.
pub fn standard_common_name(index: usize, links: usize) -> String {
    if index == 0 {
        LEAF_COMMON_NAME.to_string()
    } else if index + 1 >= links {
        "Metis test root".to_string()
    } else {
        format!("Metis test intermediate {index}")
    }
}

/// One spec per link for a standard chain of `links` links: the first link is an
/// end entity, the rest authorise another key, and the anchor names itself as
/// its own issuer.
pub fn standard_specs(links: usize) -> Vec<LinkSpec> {
    (0..links)
        .map(|index| LinkSpec {
            common_name: standard_common_name(index, links),
            issuer_common_name: standard_common_name(index + 1, links),
            ca: Some(index > 0),
            extensions: Vec::new(),
            validity: (NOT_BEFORE, NOT_AFTER),
            serial: index as u32 + 1,
        })
        .collect()
}

/// One built link: its key and its certificate's DER bytes.
pub struct TestLink {
    key: SigningKey,
    der: Vec<u8>,
}

impl TestLink {
    /// The link's signing key.
    pub fn key(&self) -> &SigningKey {
        &self.key
    }

    /// The link's raw public key.
    pub fn public_key(&self) -> [u8; 32] {
        public_key(&self.key)
    }

    /// The link's certificate, DER encoded.
    pub fn der(&self) -> &[u8] {
        &self.der
    }
}

/// A built chain: the leaf-authorising certificate first, the anchor last.
pub struct TestChain {
    links: Vec<TestLink>,
}

impl TestChain {
    /// Builds a chain of `links` links, one per spec, leaf first. `seeds[i]` is
    /// the seed of the link at `i`. A link's certificate is signed by the key of
    /// the link above it, and the anchor is signed by its own key.
    pub fn build(specs: &[LinkSpec], seeds: &[u8]) -> TestChain {
        assert_eq!(specs.len(), seeds.len(), "one seed per link");
        assert!(!specs.is_empty(), "a chain has at least one link");
        let keys: Vec<SigningKey> = seeds.iter().copied().map(key).collect();
        let mut links = Vec::new();
        for (index, spec) in specs.iter().enumerate() {
            let issuer = keys
                .get(index + 1)
                .unwrap_or_else(|| &keys[index])
                .clone();
            let certificate = build_certificate(spec, &keys[index], &issuer);
            let der = certificate.to_der().expect("a built certificate encodes");
            links.push(TestLink {
                key: keys[index].clone(),
                der,
            });
        }
        TestChain { links }
    }

    /// A standard chain of `links` links, whose seeds are `1..=links`.
    pub fn standard(links: usize) -> TestChain {
        let seeds: Vec<u8> = (1..=links as u8).collect();
        TestChain::build(&standard_specs(links), &seeds)
    }

    /// Every link, leaf first.
    pub fn links(&self) -> &[TestLink] {
        &self.links
    }

    /// The link at `index`.
    pub fn link(&self, index: usize) -> &TestLink {
        &self.links[index]
    }

    /// The leaf-authorising link, whose key signs the payload.
    pub fn leaf(&self) -> &TestLink {
        &self.links[0]
    }

    /// The anchor link, whose key id must be in the trust store's root set.
    pub fn anchor(&self) -> &TestLink {
        &self.links[self.links.len() - 1]
    }

    /// The chain region: the links' certificates concatenated, leaf first.
    pub fn der(&self) -> Vec<u8> {
        self.links
            .iter()
            .flat_map(|link| link.der.iter().copied())
            .collect()
    }
}

/// The fixed container header, written big-endian at the documented offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub container_version: u16,
    pub signing_suite: u16,
    pub header_length: u16,
    pub reserved: u16,
    pub chain_length: u32,
    pub signature_offset: u64,
    pub signature_length: u32,
    pub zip_length: u64,
}

impl Header {
    /// The header a correct producer writes for regions of these lengths.
    pub fn standard(chain_length: usize, signature_length: usize, zip_length: usize) -> Header {
        Header {
            container_version: CONTAINER_VERSION,
            signing_suite: SIGNING_SUITE,
            header_length: HEADER_LENGTH,
            reserved: 0,
            chain_length: chain_length as u32,
            signature_offset: u64::from(HEADER_LENGTH) + chain_length as u64,
            signature_length: signature_length as u32,
            zip_length: zip_length as u64,
        }
    }

    /// The header's 40 bytes.
    pub fn to_bytes(self) -> [u8; 40] {
        let mut bytes = [0u8; 40];
        bytes[0..8].copy_from_slice(&MAGIC);
        bytes[8..10].copy_from_slice(&self.container_version.to_be_bytes());
        bytes[10..12].copy_from_slice(&self.signing_suite.to_be_bytes());
        bytes[12..14].copy_from_slice(&self.header_length.to_be_bytes());
        bytes[14..16].copy_from_slice(&self.reserved.to_be_bytes());
        bytes[16..20].copy_from_slice(&self.chain_length.to_be_bytes());
        bytes[20..28].copy_from_slice(&self.signature_offset.to_be_bytes());
        bytes[28..32].copy_from_slice(&self.signature_length.to_be_bytes());
        bytes[32..40].copy_from_slice(&self.zip_length.to_be_bytes());
        bytes
    }
}

/// The message the payload signature covers: the domain tag, the header bytes,
/// the chain bytes, then the archive bytes, with the signature itself deleted
/// rather than zero-filled.
pub fn signed_message(header: &[u8; 40], chain: &[u8], archive: &[u8]) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(DOMAIN_TAG);
    message.extend_from_slice(header);
    message.extend_from_slice(chain);
    message.extend_from_slice(archive);
    message
}

/// Signs a header, a chain and an archive with `key` and lays the container out:
/// header, chain, signature, archive.
pub fn bundle(header: &Header, chain: &[u8], archive: &[u8], key: &SigningKey) -> Vec<u8> {
    let header_bytes = header.to_bytes();
    let signature = key.sign(&signed_message(&header_bytes, chain, archive));
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&header_bytes);
    bytes.extend_from_slice(chain);
    bytes.extend_from_slice(&signature.to_bytes());
    bytes.extend_from_slice(archive);
    bytes
}

/// The standard bundle for a chain and an archive: a correct header, one
/// 64-byte signature by the leaf key, and the archive.
pub fn signed_bundle(chain: &TestChain, archive: &[u8]) -> Vec<u8> {
    let chain_bytes = chain.der();
    let header = Header::standard(chain_bytes.len(), 64, archive.len());
    bundle(&header, &chain_bytes, archive, chain.leaf().key())
}

/// A certificate's profile: the names, the `basicConstraints` and any extra
/// extensions. The project owns a profile this small on purpose, so the
/// verifier's checks are about that profile and not about TLS's.
struct TestProfile {
    subject: Name,
    issuer: Name,
    ca: Option<bool>,
    extensions: Vec<Extension>,
}

impl BuilderProfile for TestProfile {
    fn get_issuer(&self, _subject: &Name) -> Name {
        self.issuer.clone()
    }

    fn get_subject(&self) -> Name {
        self.subject.clone()
    }

    fn build_extensions(
        &self,
        _spk: SubjectPublicKeyInfoRef<'_>,
        _issuer_spk: SubjectPublicKeyInfoRef<'_>,
        _tbs: &TbsCertificate,
    ) -> x509_cert::builder::Result<Vec<Extension>> {
        let mut extensions = Vec::new();
        if let Some(ca) = self.ca {
            let value = BasicConstraints {
                ca,
                path_len_constraint: None,
            }
            .to_der()?;
            extensions.push(Extension {
                extn_id: BasicConstraints::OID,
                critical: true,
                extn_value: OctetString::new(value)?,
            });
        }
        extensions.extend(self.extensions.iter().cloned());
        Ok(extensions)
    }
}

/// Builds one link's certificate: `subject_key`'s public key is the subject, and
/// `issuer_key` signs it.
fn build_certificate(
    spec: &LinkSpec,
    subject_key: &SigningKey,
    issuer_key: &SigningKey,
) -> Certificate {
    let profile = TestProfile {
        subject: name(&spec.common_name),
        issuer: name(&spec.issuer_common_name),
        ca: spec.ca,
        extensions: spec.extensions.clone(),
    };
    let public_key = SubjectPublicKeyInfo::from_key(&subject_key.verifying_key())
        .expect("an ed25519 public key encodes");
    let validity = Validity::new(time_at(spec.validity.0), time_at(spec.validity.1));
    let builder = CertificateBuilder::new(
        profile,
        SerialNumber::from(spec.serial),
        validity,
        public_key,
    )
    .expect("the specification builds a certificate");
    builder
        .build::<_, ed25519_dalek::Signature>(issuer_key)
        .expect("the certificate signs")
}

/// A distinguished name carrying one common name.
fn name(common_name: &str) -> Name {
    Name::from_str(&format!("CN={common_name}")).expect("a common name parses")
}

/// A certificate time from unix seconds.
fn time_at(unix_seconds: i64) -> Time {
    let seconds = u64::try_from(unix_seconds).expect("a time after the unix epoch");
    DateTime::from_unix_duration(Duration::from_secs(seconds))
        .expect("a representable date")
        .into()
}
