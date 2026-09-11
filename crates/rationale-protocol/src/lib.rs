//! Canonical JSON support for the versioned Rationale proof protocol.

use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;

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

    use super::to_canonical_json;

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
}
