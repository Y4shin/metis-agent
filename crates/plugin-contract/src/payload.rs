//! The payload envelope (D4): bytes whose first byte is a header.
//!
//! Every payload that crosses the plugin boundary, in both directions, is a
//! byte string whose first byte carries three fields:
//!
//! ```text
//! bit  7 6 5 | 4     | 3 2 1 0
//!      reserved | codec | format version
//! ```
//!
//! The low four bits are the payload format version, bit 4 selects the codec
//! (`0` is UTF-8 JSON, `1` is MessagePack), and bits 5 to 7 are reserved and
//! must be zero. The ABI length field is unchanged: the header is the first
//! byte of the buffer, not a field of a struct.
//!
//! The constants below are part of the generated header, because an envelope a
//! non-Rust implementer has to reproduce belongs next to the tables and not in
//! a plugin author's head. The rules themselves (the header accessors, the
//! validator, encode and strip) are Rust-only: `cbindgen` emits constants, and
//! a function without `#[no_mangle]` is not rendered.
//!
//! Host and plugin share this one implementation of the rules, which is what
//! keeps the two sides from drifting.

use crate::abi::MetisStatus;

/// Mask for the low four bits: the payload format version.
pub const METIS_PAYLOAD_VERSION_MASK: u8 = 0x0f;
/// Bit 4: the codec selector.
pub const METIS_PAYLOAD_CODEC_BIT: u8 = 0x10;
/// Bits 5 to 7: reserved, must be zero.
pub const METIS_PAYLOAD_RESERVED_MASK: u8 = 0xe0;
/// The only payload format version defined at first.
pub const METIS_PAYLOAD_FORMAT_VERSION: u8 = 1;
/// Codec 0: UTF-8 JSON. The only codec this build implements.
pub const METIS_CODEC_JSON: u8 = 0;
/// Codec 1: MessagePack. Defined by D4 so it can arrive without a format
/// break, not implemented by this build, and therefore refused as unsupported.
pub const METIS_CODEC_MESSAGEPACK: u8 = 1;

/// Builds the one-byte payload header from a format version and a codec.
pub const fn metis_payload_header(format_version: u8, codec: u8) -> u8 {
    (format_version & METIS_PAYLOAD_VERSION_MASK) | ((codec & 1) << 4)
}

/// The low four bits of a header: the payload format version.
pub const fn metis_payload_format_version(header: u8) -> u8 {
    header & METIS_PAYLOAD_VERSION_MASK
}

/// Bit 4 of a header: the codec selector.
pub const fn metis_payload_codec(header: u8) -> u8 {
    (header & METIS_PAYLOAD_CODEC_BIT) >> 4
}

/// Bits 5 to 7 of a header: reserved, zero in a valid header.
pub const fn metis_payload_reserved(header: u8) -> u8 {
    header & METIS_PAYLOAD_RESERVED_MASK
}

/// Validates a payload header.
///
/// The two failure modes stay distinguishable on purpose: a header this build
/// could read but cannot parse (a bad format version, or a reserved bit that is
/// set) is `InvalidArgument`, while a well-formed header naming a codec this
/// build does not implement is `Unsupported`.
pub fn metis_validate_payload_header(header: u8) -> Result<(), MetisStatus> {
    if metis_payload_reserved(header) != 0 {
        return Err(MetisStatus::InvalidArgument);
    }
    if metis_payload_format_version(header) != METIS_PAYLOAD_FORMAT_VERSION {
        return Err(MetisStatus::InvalidArgument);
    }
    if metis_payload_codec(header) != METIS_CODEC_JSON {
        return Err(MetisStatus::Unsupported);
    }
    Ok(())
}

/// Encodes a body with the version 1 JSON header this build writes.
pub fn metis_encode_payload(body: &[u8]) -> Vec<u8> {
    metis_encode_payload_with(
        metis_payload_header(METIS_PAYLOAD_FORMAT_VERSION, METIS_CODEC_JSON),
        body,
    )
}

/// Encodes a body with an arbitrary header byte.
///
/// This exists for negative tests and for a future codec: it is the only way to
/// build a payload whose header the receiving side must reject.
pub fn metis_encode_payload_with(header: u8, body: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(body.len() + 1);
    encoded.push(header);
    encoded.extend_from_slice(body);
    encoded
}

/// Strips and validates the header of a received payload, returning the body.
///
/// An empty payload has no header byte at all, which is an invalid argument
/// rather than an empty body.
pub fn metis_payload_body(bytes: &[u8]) -> Result<&[u8], MetisStatus> {
    match bytes.split_first() {
        Some((&header, body)) => {
            metis_validate_payload_header(header)?;
            Ok(body)
        }
        None => Err(MetisStatus::InvalidArgument),
    }
}
