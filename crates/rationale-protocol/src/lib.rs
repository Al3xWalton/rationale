//! Canonical JSON support for the versioned Rationale proof protocol.

use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;

mod worker;

pub use worker::{ProofUnavailable, ProofUnavailableReason, WorkerConfig, WorkerSupervisor};

/// Number of bytes in the big-endian worker frame header.
pub const FRAME_HEADER_BYTES: usize = 4;

/// Failure to decode one complete length-prefixed worker frame.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum FrameDecodeError {
    /// The input ended before the fixed-width length header.
    #[error("worker frame header is incomplete")]
    IncompleteHeader,
    /// The declared payload exceeds the caller's allocation bound.
    #[error("worker frame length {length} exceeds limit {limit}")]
    FrameTooLarge {
        /// Declared payload length.
        length: usize,
        /// Configured payload limit.
        limit: usize,
    },
    /// The input ended before the declared payload length.
    #[error("worker frame payload is incomplete: declared {declared}, available {available}")]
    IncompletePayload {
        /// Declared payload length.
        declared: usize,
        /// Bytes available after the header.
        available: usize,
    },
    /// A complete frame was followed by unexpected bytes.
    #[error("worker frame has {trailing} trailing bytes")]
    TrailingBytes {
        /// Bytes remaining after the declared payload.
        trailing: usize,
    },
}

/// Decode exactly one bounded length-prefixed worker frame without allocating.
///
/// # Errors
///
/// Returns a typed error for incomplete headers or payloads, oversized declared
/// payloads, and trailing bytes.
pub fn decode_frame(frame: &[u8], max_payload: usize) -> Result<&[u8], FrameDecodeError> {
    let header: [u8; FRAME_HEADER_BYTES] = frame
        .get(..FRAME_HEADER_BYTES)
        .ok_or(FrameDecodeError::IncompleteHeader)?
        .try_into()
        .map_err(|_| FrameDecodeError::IncompleteHeader)?;
    let length = checked_frame_length(header, max_payload)?;
    let available = frame.len() - FRAME_HEADER_BYTES;
    if available < length {
        return Err(FrameDecodeError::IncompletePayload {
            declared: length,
            available,
        });
    }
    if available > length {
        return Err(FrameDecodeError::TrailingBytes {
            trailing: available - length,
        });
    }
    Ok(&frame[FRAME_HEADER_BYTES..])
}

fn checked_frame_length(
    header: [u8; FRAME_HEADER_BYTES],
    max_payload: usize,
) -> Result<usize, FrameDecodeError> {
    let length = u32::from_be_bytes(header) as usize;
    if length > max_payload {
        Err(FrameDecodeError::FrameTooLarge {
            length,
            limit: max_payload,
        })
    } else {
        Ok(length)
    }
}

/// Failure to encode a protocol value as canonical JSON.
#[derive(Debug, Error)]
pub enum CanonicalJsonError {
    /// Serde could not convert the typed value into JSON.
    #[error("failed to encode protocol JSON: {0}")]
    Encode(#[from] serde_json::Error),
}

/// Serialize a protocol value with recursively sorted object keys.
///
/// Array order is semantic and must already follow the protocol's canonical
/// ordering rules.
///
/// # Errors
///
/// Returns an error if Serde cannot encode the supplied value.
pub fn to_canonical_json<T: Serialize>(value: &T) -> Result<String, CanonicalJsonError> {
    let mut json = serde_json::to_value(value)?;
    canonicalize_value(&mut json);
    Ok(serde_json::to_string(&json)?)
}

fn canonicalize_value(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                canonicalize_value(item);
            }
        }
        Value::Object(object) => {
            let previous = std::mem::take(object);
            let mut entries: Vec<_> = previous.into_iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));

            let mut sorted = Map::new();
            for (key, mut child) in entries {
                canonicalize_value(&mut child);
                sorted.insert(key, child);
            }
            *object = sorted;
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use jsonschema::validator_for;
    use rationale_model::{KernelRequest, KernelResponse};
    use serde::de::DeserializeOwned;
    use serde_json::Value;

    use super::{FrameDecodeError, decode_frame, to_canonical_json};

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/protocol")
            .join(name)
    }

    fn schema(name: &str) -> Value {
        let source = fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../protocol/v1")
                .join(name),
        )
        .expect("schema should be readable");
        serde_json::from_str(&source).expect("schema should be valid JSON")
    }

    fn assert_round_trip<T: DeserializeOwned + SerializeFixture>(name: &str) {
        let pretty = fs::read_to_string(fixture(&format!("{name}.json")))
            .expect("fixture should be readable");
        let parsed: T = serde_json::from_str(&pretty).expect("fixture should match Rust types");
        let canonical = to_canonical_json(&parsed).expect("fixture should serialize");
        let expected = fs::read_to_string(fixture(&format!("{name}.canonical.json")))
            .expect("canonical fixture should be readable");
        assert_eq!(canonical, expected.trim_end());
    }

    trait SerializeFixture: serde::Serialize {}

    impl<T: serde::Serialize> SerializeFixture for T {}

    #[test]
    fn request_matches_schema_and_canonical_fixture() {
        let source = fs::read_to_string(fixture("request-established.json"))
            .expect("fixture should be readable");
        let instance: Value = serde_json::from_str(&source).expect("fixture should be JSON");
        let validator =
            validator_for(&schema("request.schema.json")).expect("schema should compile");
        assert!(validator.is_valid(&instance));
        assert_round_trip::<KernelRequest>("request-established");
    }

    #[test]
    fn responses_match_schema_and_canonical_fixtures() {
        let response_schema = schema("response.schema.json");
        let validator = validator_for(&response_schema).expect("schema should compile");

        for name in [
            "response-established",
            "response-partial",
            "response-not-established",
            "response-conflicted",
            "response-error",
        ] {
            let source = fs::read_to_string(fixture(&format!("{name}.json")))
                .expect("fixture should be readable");
            let instance: Value = serde_json::from_str(&source).expect("fixture should be JSON");
            assert!(validator.is_valid(&instance), "schema rejected {name}");
            assert_round_trip::<KernelResponse>(name);
        }
    }

    #[test]
    fn standalone_error_matches_schema() {
        let source =
            fs::read_to_string(fixture("response-error.json")).expect("fixture should be readable");
        let response: Value = serde_json::from_str(&source).expect("fixture should be JSON");
        let error = response
            .get("error")
            .expect("fixture should contain an error");
        let validator = validator_for(&schema("error.schema.json")).expect("schema should compile");
        assert!(validator.is_valid(error));
    }

    #[test]
    fn frame_decoder_enforces_declared_length_and_bound() {
        assert_eq!(decode_frame(&[0, 0, 0, 2, b'o', b'k'], 2), Ok(&b"ok"[..]));
        assert_eq!(
            decode_frame(&[0, 0, 0, 3, b'n', b'o'], 3),
            Err(FrameDecodeError::IncompletePayload {
                declared: 3,
                available: 2,
            })
        );
        assert_eq!(
            decode_frame(&[0, 0, 0, 2, b'o', b'k'], 1),
            Err(FrameDecodeError::FrameTooLarge {
                length: 2,
                limit: 1,
            })
        );
        assert_eq!(
            decode_frame(&[0, 0, 0, 1, b'o', b'k'], 2),
            Err(FrameDecodeError::TrailingBytes { trailing: 1 })
        );
    }
}
