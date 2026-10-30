//! Producer-side conversion from the isolated gRPC read model to neutral schema v1.
//!
//! This module remains in the gRPC dependency domain. It does not import the legacy
//! Sui SDK and does not construct transaction arguments.

use crate::{SuiObjectRecord, SuiOwner};
use anyhow::{Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sui_neutral_object_boundary::{NeutralObjectRecord, NeutralOwner, SCHEMA_VERSION};

/// Convert a validated gRPC read record to the dependency-neutral schema.
///
/// Hex identifiers are normalized to the schema's canonical 0x-prefixed lowercase form;
/// the chain ID, digest, type, versions, owner metadata and BCS bytes are otherwise preserved.
pub fn to_neutral_record(record: &SuiObjectRecord) -> Result<NeutralObjectRecord> {
    if record.chain_id.trim().is_empty() || record.chain_id.trim() != record.chain_id {
        bail!("gRPC record has invalid chain identity");
    }
    if record.version == 0 {
        bail!("gRPC record has object version zero");
    }
    if record.digest.trim().is_empty() || record.digest.trim() != record.digest {
        bail!("gRPC record has invalid digest");
    }
    if record.object_type.trim().is_empty() || record.object_type.trim() != record.object_type {
        bail!("gRPC record has invalid object type");
    }

    let owner = match &record.owner {
        SuiOwner::Address { address } => NeutralOwner::Address {
            address: canonical_id(address)?,
        },
        SuiOwner::Object { object_id } => NeutralOwner::Object {
            object_id: canonical_id(object_id)?,
        },
        SuiOwner::Shared {
            initial_shared_version,
        } => {
            if *initial_shared_version == 0 {
                bail!("gRPC record has shared initial version zero");
            }
            NeutralOwner::Shared {
                initial_shared_version: *initial_shared_version,
            }
        }
        SuiOwner::Immutable => NeutralOwner::Immutable,
        SuiOwner::ConsensusAddress {
            address,
            start_version,
        } => {
            if *start_version == 0 {
                bail!("gRPC record has consensus start version zero");
            }
            NeutralOwner::ConsensusV2 {
                address: canonical_id(address)?,
                start_version: *start_version,
            }
        }
    };

    let neutral = NeutralObjectRecord {
        schema_version: SCHEMA_VERSION,
        chain_id: record.chain_id.clone(),
        object_id: canonical_id(&record.object_id)?,
        version: record.version,
        digest: record.digest.clone(),
        object_type: record.object_type.clone(),
        owner,
        contents_bcs_base64: record
            .contents_bcs
            .as_ref()
            .map(|bytes| STANDARD.encode(bytes)),
    };
    neutral
        .validate()
        .map_err(|error| anyhow!("neutral schema validation failed: {error}"))?;
    Ok(neutral)
}

/// Serialize the producer record as schema-v1 JSON after structural validation.
pub fn to_neutral_json(record: &SuiObjectRecord) -> Result<String> {
    let neutral = to_neutral_record(record)?;
    serde_json::to_string(&neutral).map_err(Into::into)
}

fn canonical_id(value: &str) -> Result<String> {
    let hex = value.strip_prefix("0x").unwrap_or(value);
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("gRPC record contains a malformed 32-byte identifier");
    }
    Ok(format!("0x{}", hex.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SuiOwner;

    const ID: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ADDRESS: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn record(owner: SuiOwner) -> SuiObjectRecord {
        SuiObjectRecord {
            chain_id: "pinned-test-chain-genesis-digest".to_owned(),
            object_id: ID.to_owned(),
            version: 42,
            digest: "Base58DigestPreservedExactly".to_owned(),
            object_type: "0x2::example::Thing".to_owned(),
            owner,
            contents_bcs: None,
        }
    }

    #[test]
    fn maps_all_owner_variants_without_metadata_loss() {
        let cases = [
            (
                SuiOwner::Address {
                    address: ADDRESS.to_owned(),
                },
                NeutralOwner::Address {
                    address: ADDRESS.to_owned(),
                },
            ),
            (
                SuiOwner::Object {
                    object_id: ADDRESS.to_owned(),
                },
                NeutralOwner::Object {
                    object_id: ADDRESS.to_owned(),
                },
            ),
            (
                SuiOwner::Shared {
                    initial_shared_version: 7,
                },
                NeutralOwner::Shared {
                    initial_shared_version: 7,
                },
            ),
            (SuiOwner::Immutable, NeutralOwner::Immutable),
            (
                SuiOwner::ConsensusAddress {
                    address: ADDRESS.to_owned(),
                    start_version: 9,
                },
                NeutralOwner::ConsensusV2 {
                    address: ADDRESS.to_owned(),
                    start_version: 9,
                },
            ),
        ];
        for (source_owner, expected_owner) in cases {
            let source = record(source_owner);
            let mapped = to_neutral_record(&source).unwrap();
            assert_eq!(mapped.schema_version, SCHEMA_VERSION);
            assert_eq!(mapped.chain_id, source.chain_id);
            assert_eq!(mapped.object_id, source.object_id);
            assert_eq!(mapped.version, source.version);
            assert_eq!(mapped.digest, source.digest);
            assert_eq!(mapped.object_type, source.object_type);
            assert_eq!(mapped.owner, expected_owner);
            mapped.validate().unwrap();
        }
    }

    #[test]
    fn encodes_optional_bcs_payload_and_json_round_trips() {
        let mut source = record(SuiOwner::Immutable);
        source.contents_bcs = Some(vec![0, 1, 2, 253, 254, 255]);
        let json = to_neutral_json(&source).unwrap();
        let decoded: NeutralObjectRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.contents_bcs_base64.as_deref(), Some("AAEC/f7/"));
        assert_eq!(
            STANDARD
                .decode(decoded.contents_bcs_base64.unwrap())
                .unwrap(),
            source.contents_bcs.unwrap()
        );
    }

    #[test]
    fn canonicalizes_hex_identifiers_without_mutating_other_fields() {
        let mut source = record(SuiOwner::Address {
            address: format!("0x{}", "BB".repeat(32)),
        });
        source.object_id = "0x".to_owned() + &"AA".repeat(32);
        let mapped = to_neutral_record(&source).unwrap();
        assert_eq!(mapped.object_id, ID);
        assert_eq!(
            mapped.owner,
            NeutralOwner::Address {
                address: ADDRESS.to_owned()
            }
        );
        assert_eq!(mapped.digest, source.digest);
    }

    #[test]
    fn rejects_invalid_source_records_before_serialization() {
        let mut source = record(SuiOwner::Immutable);
        source.version = 0;
        assert!(to_neutral_record(&source).is_err());
        let source = record(SuiOwner::Shared {
            initial_shared_version: 0,
        });
        assert!(to_neutral_record(&source).is_err());
        let mut source = record(SuiOwner::Immutable);
        source.object_id = "0x1234".to_owned();
        assert!(to_neutral_record(&source).is_err());
    }
}
