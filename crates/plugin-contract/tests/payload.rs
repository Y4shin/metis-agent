//! Envelope seam: the one-byte payload header (D4) and its validation rules.
//!
//! These tests live at the crate's public boundary, because that is where the
//! host, the fixture plugin, and every later consumer meet the envelope. The
//! same rules are exercised across the real ABI boundary by the fixture
//! harness (`crates/plugin-fixture/src/bin/abi-harness.rs`).

use plugin_contract::abi::MetisStatus;
use plugin_contract::payload::*;

#[test]
fn header_bits_are_version_then_codec_then_reserved() {
    let header = metis_payload_header(METIS_PAYLOAD_FORMAT_VERSION, METIS_CODEC_JSON);
    assert_eq!(header, 0x01, "version 1 with the JSON codec is 0x01");
    assert_eq!(
        metis_payload_format_version(header),
        1,
        "low four bits carry the format version"
    );
    assert_eq!(
        metis_payload_codec(header),
        METIS_CODEC_JSON,
        "bit 4 carries the codec"
    );
    assert_eq!(
        metis_payload_reserved(header),
        0,
        "bits 5 to 7 are reserved and zero"
    );

    // The codec bit is bit 4, so codec 1 sets exactly that bit and nothing else.
    assert_eq!(
        metis_payload_header(METIS_PAYLOAD_FORMAT_VERSION, METIS_CODEC_MESSAGEPACK),
        0x11
    );
}

#[test]
fn encoding_and_body_stripping_round_trip_a_body() {
    let body = br#"{"echo":"hi"}"#;
    let encoded = metis_encode_payload(body);
    assert_eq!(
        encoded[0], 0x01,
        "the encoded payload starts with the default header"
    );
    assert_eq!(&encoded[1..], body, "the body follows the header unchanged");
    assert_eq!(
        metis_payload_body(&encoded),
        Ok(&body[..]),
        "the body strips back out"
    );
}

#[test]
fn an_empty_payload_is_invalid_argument() {
    assert_eq!(metis_payload_body(&[]), Err(MetisStatus::InvalidArgument));
}

#[test]
fn a_bad_format_version_is_invalid_argument() {
    // Version 2 is not the version this build knows.
    assert_eq!(
        metis_validate_payload_header(0x02),
        Err(MetisStatus::InvalidArgument)
    );
    // Version 0 is not a version either.
    assert_eq!(
        metis_validate_payload_header(0x00),
        Err(MetisStatus::InvalidArgument)
    );
}

#[test]
fn reserved_bits_are_invalid_argument() {
    for reserved in [0x20_u8, 0x40, 0x80, 0xe0] {
        let header =
            metis_payload_header(METIS_PAYLOAD_FORMAT_VERSION, METIS_CODEC_JSON) | reserved;
        assert_eq!(
            metis_validate_payload_header(header),
            Err(MetisStatus::InvalidArgument),
            "header {header:#04x} sets a reserved bit"
        );
    }
}

#[test]
fn a_codec_this_build_does_not_know_is_unsupported() {
    // The MessagePack codec is defined by the decision but not implemented by
    // this build, so it must stay distinguishable from a malformed header.
    let messagepack = metis_payload_header(METIS_PAYLOAD_FORMAT_VERSION, METIS_CODEC_MESSAGEPACK);
    assert_eq!(
        metis_validate_payload_header(messagepack),
        Err(MetisStatus::Unsupported)
    );
    assert_eq!(
        metis_payload_body(&[messagepack, b'{']),
        Err(MetisStatus::Unsupported)
    );
}

#[test]
fn a_valid_header_validates() {
    let header = metis_payload_header(METIS_PAYLOAD_FORMAT_VERSION, METIS_CODEC_JSON);
    assert_eq!(metis_validate_payload_header(header), Ok(()));
}
