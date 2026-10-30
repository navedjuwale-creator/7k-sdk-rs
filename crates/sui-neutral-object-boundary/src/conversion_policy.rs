//! Pure, dependency-neutral object conversion policy.
//!
//! This module returns a decision plan only. It does not construct legacy SDK
//! ObjectArg values, programmable transactions, or transaction bytes.

use crate::{NeutralObjectRecord, NeutralOwner, ValidationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversionIntent<'a> {
    /// Required for address-owned objects; must exactly match the recorded owner.
    pub expected_sender: Option<&'a str>,
    /// Required only for shared objects. Some(false) is explicit read-only;
    /// Some(true) is explicit mutable access.
    pub shared_mutable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversionPlan {
    ImmOrOwned {
        object_id: String,
        version: u64,
        digest: String,
    },
    Shared {
        object_id: String,
        initial_shared_version: u64,
        mutable: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversionError {
    InvalidRecord(ValidationError),
    ChainMismatch,
    MissingExpectedSender,
    SenderOwnerMismatch,
    MissingSharedMutability,
    IllegalSharedMutabilityIntent,
    UnsupportedObjectOwner,
    UnsupportedConsensusOwner,
}

impl std::fmt::Display for ConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ConversionError {}

/// Evaluate whether a neutral record may be represented by the supported
/// legacy object-argument *shape*. This is policy only, not SDK conversion.
pub fn plan_object_conversion(
    record: &NeutralObjectRecord,
    verified_chain_id: &str,
    intent: ConversionIntent<'_>,
) -> Result<ConversionPlan, ConversionError> {
    record
        .validate()
        .map_err(ConversionError::InvalidRecord)?;

    if record.chain_id != verified_chain_id {
        return Err(ConversionError::ChainMismatch);
    }

    match &record.owner {
        NeutralOwner::Address { address } => {
            if intent.shared_mutable.is_some() {
                return Err(ConversionError::IllegalSharedMutabilityIntent);
            }
            let sender = intent
                .expected_sender
                .ok_or(ConversionError::MissingExpectedSender)?;
            if sender != address {
                return Err(ConversionError::SenderOwnerMismatch);
            }
            Ok(owned_plan(record))
        }
        NeutralOwner::Immutable => {
            if intent.shared_mutable.is_some() {
                return Err(ConversionError::IllegalSharedMutabilityIntent);
            }
            Ok(owned_plan(record))
        }
        NeutralOwner::Shared {
            initial_shared_version,
        } => {
            if intent.expected_sender.is_some() {
                return Err(ConversionError::IllegalSharedMutabilityIntent);
            }
            let mutable = intent
                .shared_mutable
                .ok_or(ConversionError::MissingSharedMutability)?;
            Ok(ConversionPlan::Shared {
                object_id: record.object_id.clone(),
                initial_shared_version: *initial_shared_version,
                mutable,
            })
        }
        NeutralOwner::Object { .. } => Err(ConversionError::UnsupportedObjectOwner),
        NeutralOwner::ConsensusV2 { .. } => {
            Err(ConversionError::UnsupportedConsensusOwner)
        }
    }
}

fn owned_plan(record: &NeutralObjectRecord) -> ConversionPlan {
    ConversionPlan::ImmOrOwned {
        object_id: record.object_id.clone(),
        version: record.version,
        digest: record.digest.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAIN: &str = "verified-test-chain";
    const OBJECT: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ADDRESS: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn record(owner: NeutralOwner) -> NeutralObjectRecord {
        NeutralObjectRecord {
            schema_version: crate::SCHEMA_VERSION,
            chain_id: CHAIN.to_owned(),
            object_id: OBJECT.to_owned(),
            version: 42,
            digest: "11111111111111111111111111111111".to_owned(),
            object_type: "0x2::example::Thing".to_owned(),
            owner,
            contents_bcs_base64: None,
        }
    }

    fn intent() -> ConversionIntent<'static> {
        ConversionIntent {
            expected_sender: None,
            shared_mutable: None,
        }
    }

    #[test]
    fn address_owner_requires_exact_sender_match() {
        let value = record(NeutralOwner::Address { address: ADDRESS.into() });
        let plan = plan_object_conversion(
            &value,
            CHAIN,
            ConversionIntent { expected_sender: Some(ADDRESS), shared_mutable: None },
        ).unwrap();
        assert_eq!(plan, ConversionPlan::ImmOrOwned {
            object_id: OBJECT.into(), version: 42, digest: "11111111111111111111111111111111".into()
        });
    }

    #[test]
    fn address_owner_rejects_missing_or_different_sender() {
        let value = record(NeutralOwner::Address { address: ADDRESS.into() });
        assert_eq!(plan_object_conversion(&value, CHAIN, intent()), Err(ConversionError::MissingExpectedSender));
        assert_eq!(plan_object_conversion(
            &value, CHAIN,
            ConversionIntent { expected_sender: Some(OBJECT), shared_mutable: None }
        ), Err(ConversionError::SenderOwnerMismatch));
    }

    #[test]
    fn immutable_owner_has_only_owned_read_reference_shape() {
        let value = record(NeutralOwner::Immutable);
        assert!(matches!(
            plan_object_conversion(&value, CHAIN, intent()),
            Ok(ConversionPlan::ImmOrOwned { .. })
        ));
        assert_eq!(plan_object_conversion(
            &value, CHAIN,
            ConversionIntent { expected_sender: None, shared_mutable: Some(true) }
        ), Err(ConversionError::IllegalSharedMutabilityIntent));
    }

    #[test]
    fn shared_owner_requires_explicit_read_only_or_mutable_intent() {
        let value = record(NeutralOwner::Shared { initial_shared_version: 7 });
        assert_eq!(plan_object_conversion(&value, CHAIN, intent()), Err(ConversionError::MissingSharedMutability));
        assert_eq!(plan_object_conversion(
            &value, CHAIN,
            ConversionIntent { expected_sender: None, shared_mutable: Some(false) }
        ), Ok(ConversionPlan::Shared {
            object_id: OBJECT.into(), initial_shared_version: 7, mutable: false
        }));
        assert_eq!(plan_object_conversion(
            &value, CHAIN,
            ConversionIntent { expected_sender: None, shared_mutable: Some(true) }
        ), Ok(ConversionPlan::Shared {
            object_id: OBJECT.into(), initial_shared_version: 7, mutable: true
        }));
    }

    #[test]
    fn shared_owner_preserves_exact_initial_version() {
        let value = record(NeutralOwner::Shared { initial_shared_version: 987654 });
        let plan = plan_object_conversion(
            &value, CHAIN,
            ConversionIntent { expected_sender: None, shared_mutable: Some(false) }
        ).unwrap();
        assert_eq!(plan, ConversionPlan::Shared {
            object_id: OBJECT.into(), initial_shared_version: 987654, mutable: false
        });
    }

    #[test]
    fn object_owned_and_consensus_owners_fail_closed() {
        let object = record(NeutralOwner::Object { object_id: ADDRESS.into() });
        assert_eq!(plan_object_conversion(&object, CHAIN, intent()), Err(ConversionError::UnsupportedObjectOwner));
        let consensus = record(NeutralOwner::ConsensusV2 { address: ADDRESS.into(), start_version: 9 });
        assert_eq!(plan_object_conversion(&consensus, CHAIN, intent()), Err(ConversionError::UnsupportedConsensusOwner));
    }

    #[test]
    fn chain_mismatch_fails_before_owner_policy() {
        let value = record(NeutralOwner::Immutable);
        assert_eq!(plan_object_conversion(&value, "another-chain", intent()), Err(ConversionError::ChainMismatch));
    }

    #[test]
    fn non_shared_owner_rejects_shared_mutability_and_shared_rejects_sender_intent() {
        let immutable = record(NeutralOwner::Immutable);
        assert_eq!(plan_object_conversion(
            &immutable, CHAIN,
            ConversionIntent { expected_sender: None, shared_mutable: Some(false) }
        ), Err(ConversionError::IllegalSharedMutabilityIntent));

        let shared = record(NeutralOwner::Shared { initial_shared_version: 7 });
        assert_eq!(plan_object_conversion(
            &shared, CHAIN,
            ConversionIntent { expected_sender: Some(ADDRESS), shared_mutable: Some(false) }
        ), Err(ConversionError::IllegalSharedMutabilityIntent));
    }
}
