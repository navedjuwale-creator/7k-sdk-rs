//! Consumer-side parser for the dependency-neutral Sui object schema.
//!
//! This crate validates the interchange payload only. It deliberately does not
//! depend on Sui SDK/gRPC crates and does not construct transaction arguments.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use sui_neutral_object_boundary::NeutralObjectRecord;

#[derive(Debug)]
pub enum ConsumerError {
    InvalidJson(serde_json::Error),
    InvalidRecord(String),
    InvalidBase64(base64::DecodeError),
}

impl std::fmt::Display for ConsumerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(f, "invalid neutral JSON: {error}"),
            Self::InvalidRecord(error) => write!(f, "invalid neutral record: {error}"),
            Self::InvalidBase64(error) => write!(f, "invalid BCS payload base64: {error}"),
        }
    }
}

impl std::error::Error for ConsumerError {}

/// Parse schema-v1 JSON, enforce the shared structural contract, and strictly
/// validate the optional base64 payload without interpreting SDK-specific BCS.
pub fn parse_neutral_json(json: &str) -> Result<NeutralObjectRecord, ConsumerError> {
    let record: NeutralObjectRecord =
        serde_json::from_str(json).map_err(ConsumerError::InvalidJson)?;
    record
        .validate()
        .map_err(|error| ConsumerError::InvalidRecord(error.to_string()))?;
    if let Some(payload) = &record.contents_bcs_base64 {
        STANDARD
            .decode(payload)
            .map_err(ConsumerError::InvalidBase64)?;
    }
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sui_neutral_object_boundary::{NeutralOwner, SCHEMA_VERSION};

    const FIXTURE: &str =
        include_str!("../../sui-neutral-object-boundary/tests/fixtures/neutral-object-v1.json");

    #[test]
    fn parses_shared_schema_fixture_and_preserves_fields() {
        let record = parse_neutral_json(FIXTURE).unwrap();
        assert_eq!(record.schema_version, SCHEMA_VERSION);
        assert_eq!(record.chain_id, "pinned-test-chain-genesis-digest");
        assert_eq!(record.version, 42);
        assert_eq!(record.digest, "Base58DigestPreservedExactly");
        assert_eq!(
            record.owner,
            NeutralOwner::Address {
                address: format!("0x{}", "b".repeat(64))
            }
        );
        assert_eq!(record.contents_bcs_base64, None);
    }

    #[test]
    fn preserves_and_decodes_optional_contents_without_mutation() {
        let mut value: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        value["contents_bcs_base64"] = serde_json::Value::String("AAEC/f7/".to_owned());
        let encoded = serde_json::to_string(&value).unwrap();
        let record = parse_neutral_json(&encoded).unwrap();
        assert_eq!(record.contents_bcs_base64.as_deref(), Some("AAEC/f7/"));
        assert_eq!(
            STANDARD.decode(record.contents_bcs_base64.unwrap()).unwrap(),
            vec![0, 1, 2, 253, 254, 255]
        );
    }

    #[test]
    fn rejects_unsupported_schema_version() {
        let mut value: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        value["schema_version"] = serde_json::json!(2);
        assert!(parse_neutral_json(&serde_json::to_string(&value).unwrap()).is_err());
    }

    #[test]
    fn rejects_malformed_object_id() {
        let mut value: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        value["object_id"] = serde_json::Value::String("0x1234".to_owned());
        assert!(parse_neutral_json(&serde_json::to_string(&value).unwrap()).is_err());
    }

    #[test]
    fn rejects_unknown_fields_and_owner_kinds() {
        let mut value: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        value["unexpected"] = serde_json::json!(true);
        assert!(parse_neutral_json(&serde_json::to_string(&value).unwrap()).is_err());

        let mut value: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        value["owner"]["kind"] = serde_json::Value::String("future_owner".to_owned());
        assert!(parse_neutral_json(&serde_json::to_string(&value).unwrap()).is_err());
    }

    #[test]
    fn rejects_invalid_optional_base64() {
        let mut value: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        value["contents_bcs_base64"] = serde_json::Value::String("not base64!".to_owned());
        assert!(matches!(
            parse_neutral_json(&serde_json::to_string(&value).unwrap()),
            Err(ConsumerError::InvalidBase64(_))
        ));
    }

    #[test]
    fn rejects_invalid_shared_owner_metadata() {
        let mut value: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        value["owner"] = serde_json::json!({
            "kind": "shared",
            "initial_shared_version": 0
        });
        assert!(parse_neutral_json(&serde_json::to_string(&value).unwrap()).is_err());
    }
}
