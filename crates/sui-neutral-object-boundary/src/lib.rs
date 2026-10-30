//! Versioned, dependency-neutral Sui object record interchange contract.
//!
//! This crate intentionally depends on neither `sui-rpc` nor `sui-sdk`.
//! It does not construct transaction arguments or transactions.

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NeutralOwner {
    Address { address: String },
    Object { object_id: String },
    Shared { initial_shared_version: u64 },
    Immutable,
    ConsensusV2 { address: String, start_version: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeutralObjectRecord {
    pub schema_version: u16,
    /// Exact network identity verified by the producing adapter; not a display label.
    pub chain_id: String,
    pub object_id: String,
    pub version: u64,
    /// Base58-encoded 32-byte Sui object digest, preserved without transformation.
    pub digest: String,
    pub object_type: String,
    pub owner: NeutralOwner,
    /// Optional base64-encoded raw BCS contents. None means contents were not requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents_bcs_base64: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    UnsupportedSchemaVersion(u16),
    EmptyChainId,
    InvalidObjectId,
    ZeroObjectVersion,
    EmptyDigest,
    EmptyObjectType,
    InvalidOwnerAddress,
    InvalidParentObjectId,
    ZeroSharedInitialVersion,
    ZeroConsensusStartVersion,
    EmptyContents,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ValidationError {}

impl NeutralObjectRecord {
    /// Validate structural invariants without interpreting or converting SDK-specific types.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::UnsupportedSchemaVersion(
                self.schema_version,
            ));
        }
        if self.chain_id.trim().is_empty() {
            return Err(ValidationError::EmptyChainId);
        }
        if !is_object_id(&self.object_id) {
            return Err(ValidationError::InvalidObjectId);
        }
        if self.version == 0 {
            return Err(ValidationError::ZeroObjectVersion);
        }
        if self.digest.trim().is_empty() {
            return Err(ValidationError::EmptyDigest);
        }
        if self.object_type.trim().is_empty() {
            return Err(ValidationError::EmptyObjectType);
        }
        match &self.owner {
            NeutralOwner::Address { address } | NeutralOwner::ConsensusV2 { address, .. }
                if !is_object_id(address) =>
            {
                return Err(ValidationError::InvalidOwnerAddress);
            }
            NeutralOwner::Object { object_id } if !is_object_id(object_id) => {
                return Err(ValidationError::InvalidParentObjectId);
            }
            NeutralOwner::Shared {
                initial_shared_version: 0,
            } => {
                return Err(ValidationError::ZeroSharedInitialVersion);
            }
            NeutralOwner::ConsensusV2 {
                start_version: 0, ..
            } => {
                return Err(ValidationError::ZeroConsensusStartVersion);
            }
            _ => {}
        }
        if self
            .contents_bcs_base64
            .as_ref()
            .is_some_and(|value| value.is_empty())
        {
            return Err(ValidationError::EmptyContents);
        }
        Ok(())
    }
}

fn is_object_id(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("0x") else {
        return false;
    };
    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBJECT: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ADDRESS: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn record(owner: NeutralOwner) -> NeutralObjectRecord {
        NeutralObjectRecord {
            schema_version: SCHEMA_VERSION,
            chain_id: "verified-chain-id-test-fixture".to_owned(),
            object_id: OBJECT.to_owned(),
            version: 9,
            digest: "11111111111111111111111111111111".to_owned(),
            object_type: "0x2::coin::Coin<0x2::sui::SUI>".to_owned(),
            owner,
            contents_bcs_base64: None,
        }
    }

    #[test]
    fn address_owned_record_round_trips_without_field_loss() {
        let value = record(NeutralOwner::Address {
            address: ADDRESS.to_owned(),
        });
        value.validate().unwrap();
        let encoded = serde_json::to_string(&value).unwrap();
        let decoded: NeutralObjectRecord = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn all_owner_variants_round_trip() {
        let owners = [
            NeutralOwner::Address {
                address: ADDRESS.to_owned(),
            },
            NeutralOwner::Object {
                object_id: ADDRESS.to_owned(),
            },
            NeutralOwner::Shared {
                initial_shared_version: 3,
            },
            NeutralOwner::Immutable,
            NeutralOwner::ConsensusV2 {
                address: ADDRESS.to_owned(),
                start_version: 4,
            },
        ];
        for owner in owners {
            let value = record(owner);
            value.validate().unwrap();
            let decoded: NeutralObjectRecord =
                serde_json::from_str(&serde_json::to_string(&value).unwrap()).unwrap();
            assert_eq!(decoded, value);
        }
    }

    #[test]
    fn optional_bcs_payload_is_preserved_exactly() {
        let mut value = record(NeutralOwner::Immutable);
        value.contents_bcs_base64 = Some("AQIDBA==".to_owned());
        let decoded: NeutralObjectRecord =
            serde_json::from_str(&serde_json::to_string(&value).unwrap()).unwrap();
        assert_eq!(decoded.contents_bcs_base64.as_deref(), Some("AQIDBA=="));
    }

    #[test]
    fn rejects_zero_versions_and_malformed_ids() {
        let mut value = record(NeutralOwner::Immutable);
        value.version = 0;
        assert_eq!(value.validate(), Err(ValidationError::ZeroObjectVersion));
        let mut value = record(NeutralOwner::Immutable);
        value.object_id = "0x1234".to_owned();
        assert_eq!(value.validate(), Err(ValidationError::InvalidObjectId));
    }

    #[test]
    fn rejects_invalid_owner_metadata() {
        let bad_address = record(NeutralOwner::Address {
            address: "not-an-address".to_owned(),
        });
        assert_eq!(
            bad_address.validate(),
            Err(ValidationError::InvalidOwnerAddress)
        );
        let bad_shared = record(NeutralOwner::Shared {
            initial_shared_version: 0,
        });
        assert_eq!(
            bad_shared.validate(),
            Err(ValidationError::ZeroSharedInitialVersion)
        );
        let bad_consensus = record(NeutralOwner::ConsensusV2 {
            address: ADDRESS.to_owned(),
            start_version: 0,
        });
        assert_eq!(
            bad_consensus.validate(),
            Err(ValidationError::ZeroConsensusStartVersion)
        );
    }

    #[test]
    fn rejects_unknown_schema_versions_and_unknown_json_fields() {
        let mut value = record(NeutralOwner::Immutable);
        value.schema_version = 2;
        assert_eq!(
            value.validate(),
            Err(ValidationError::UnsupportedSchemaVersion(2))
        );
        let json = r#"{"schema_version":1,"chain_id":"x","object_id":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","version":1,"digest":"d","object_type":"T","owner":{"kind":"immutable"},"unexpected":true}"#;
        assert!(serde_json::from_str::<NeutralObjectRecord>(json).is_err());
    }

    #[test]
    fn rejects_empty_identity_fields_and_empty_contents() {
        let mut value = record(NeutralOwner::Immutable);
        value.chain_id.clear();
        assert_eq!(value.validate(), Err(ValidationError::EmptyChainId));
        let mut value = record(NeutralOwner::Immutable);
        value.contents_bcs_base64 = Some(String::new());
        assert_eq!(value.validate(), Err(ValidationError::EmptyContents));
    }
}
