#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

/// A length prefix over the cap is rejected without allocating the payload.
#[test]
fn oversized_prefix_is_protocol_error() {
    let mut reader = FrameReader::new();
    // Prefix claims MAX_FRAME_BYTES + 1 bytes; no payload supplied.
    let bogus = (MAX_FRAME_BYTES as u64 + 1) as u32;
    reader.push(&bogus.to_be_bytes());
    let err = reader
        .next_frame()
        .expect_err("oversized prefix must error");
    assert_eq!(err.kind(), ErrorKind::InvalidData);
}

/// A frame delivered in two halves reassembles into the original payload.
#[test]
fn split_frame_reassembles() {
    let payload = b"hello frame";
    let mut wire = Vec::new();
    wire.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    wire.extend_from_slice(payload);

    let mut reader = FrameReader::new();
    // First half: only part of the prefix+payload.
    reader.push(&wire[..3]);
    assert!(reader.next_frame().unwrap().is_none(), "partial → None");
    // Second half completes the frame.
    reader.push(&wire[3..]);
    let got = reader.next_frame().unwrap().expect("frame completes");
    assert_eq!(got, payload);
    // Buffer drained: no second frame.
    assert!(reader.next_frame().unwrap().is_none());
}

/// Two frames coalesced in one push are yielded one at a time, in order.
#[test]
fn coalesced_frames_yield_in_order() {
    let mut wire = Vec::new();
    for p in [b"one".as_slice(), b"twotwo".as_slice()] {
        wire.extend_from_slice(&(p.len() as u32).to_be_bytes());
        wire.extend_from_slice(p);
    }
    let mut reader = FrameReader::new();
    reader.push(&wire);
    assert_eq!(reader.next_frame().unwrap().unwrap(), b"one");
    assert_eq!(reader.next_frame().unwrap().unwrap(), b"twotwo");
    assert!(reader.next_frame().unwrap().is_none());
}
