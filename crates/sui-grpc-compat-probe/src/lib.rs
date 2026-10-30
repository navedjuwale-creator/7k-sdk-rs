use anyhow::{Result, bail};
use std::str::FromStr;
use sui_types::{
    base_types::{ObjectID, SequenceNumber, SuiAddress},
    digests::ObjectDigest,
    transaction::ObjectArg,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NeutralOwner {
    Address(String),
    Immutable,
    Shared { initial_shared_version: u64 },
    Object(String),
    ConsensusV2 { address: String, start_version: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeutralObjectRef {
    pub object_id: String,
    pub version: u64,
    pub digest: String,
    pub owner: NeutralOwner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedAccess {
    ReadOnly,
    Mutable,
}

/// Pure conversion only. This does not construct a ProgrammableTransaction or sign anything.
pub fn to_legacy_object_arg(
    record: &NeutralObjectRef,
    sender: Option<&str>,
    shared_access: Option<SharedAccess>,
) -> Result<ObjectArg> {
    let id = ObjectID::from_str(&record.object_id)
        .map_err(|e| anyhow::anyhow!("invalid object ID: {e}"))?;
    if record.version == 0 {
        bail!("object version must be nonzero");
    }
    let version = SequenceNumber::from_u64(record.version);
    let digest = ObjectDigest::from_str(&record.digest)
        .map_err(|e| anyhow::anyhow!("invalid object digest: {e}"))?;
    let object_ref = (id, version, digest);

    match &record.owner {
        NeutralOwner::Address(owner) => {
            if shared_access.is_some() {
                bail!("shared access is valid only for shared objects");
            }
            let owner = SuiAddress::from_str(owner)
                .map_err(|e| anyhow::anyhow!("invalid address owner: {e}"))?;
            let sender = sender.ok_or_else(|| {
                anyhow::anyhow!("address-owned conversion requires an explicit sender")
            })?;
            let sender = SuiAddress::from_str(sender)
                .map_err(|e| anyhow::anyhow!("invalid sender address: {e}"))?;
            if owner != sender {
                bail!("address-owned object owner does not match explicit sender");
            }
            Ok(ObjectArg::ImmOrOwnedObject(object_ref))
        }
        NeutralOwner::Immutable => {
            if shared_access.is_some() {
                bail!("shared access is valid only for shared objects");
            }
            Ok(ObjectArg::ImmOrOwnedObject(object_ref))
        }
        NeutralOwner::Shared {
            initial_shared_version,
        } => {
            if *initial_shared_version == 0 {
                bail!("shared initial version must be nonzero");
            }
            let access = shared_access.ok_or_else(|| {
                anyhow::anyhow!("shared conversion requires explicit access intent")
            })?;
            Ok(ObjectArg::SharedObject {
                id,
                initial_shared_version: SequenceNumber::from_u64(*initial_shared_version),
                mutable: matches!(access, SharedAccess::Mutable),
            })
        }
        NeutralOwner::Object(_) => {
            bail!("object-owned conversion is unsupported pending Receiving semantics review")
        }
        NeutralOwner::ConsensusV2 { .. } => bail!(
            "ConsensusV2 conversion is unsupported because ObjectArg has no authenticator field"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ADDRESS: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const DIGEST: &str = "11111111111111111111111111111111";

    fn record(owner: NeutralOwner) -> NeutralObjectRef {
        NeutralObjectRef {
            object_id: ID.to_owned(),
            version: 7,
            digest: DIGEST.to_owned(),
            owner,
        }
    }

    #[test]
    fn address_owned_requires_exact_sender_match() {
        let value = record(NeutralOwner::Address(ADDRESS.to_owned()));
        assert!(matches!(
            to_legacy_object_arg(&value, Some(ADDRESS), None).unwrap(),
            ObjectArg::ImmOrOwnedObject(_)
        ));
        assert!(to_legacy_object_arg(&value, None, None).is_err());
        assert!(to_legacy_object_arg(&value, Some(ID), None).is_err());
    }

    #[test]
    fn immutable_preserves_exact_object_reference() {
        let value = record(NeutralOwner::Immutable);
        assert!(matches!(
            to_legacy_object_arg(&value, None, None).unwrap(),
            ObjectArg::ImmOrOwnedObject(_)
        ));
    }

    #[test]
    fn shared_requires_explicit_access_and_preserves_initial_version() {
        let value = record(NeutralOwner::Shared {
            initial_shared_version: 3,
        });
        assert!(to_legacy_object_arg(&value, None, None).is_err());
        match to_legacy_object_arg(&value, None, Some(SharedAccess::ReadOnly)).unwrap() {
            ObjectArg::SharedObject {
                initial_shared_version,
                mutable,
                ..
            } => {
                assert_eq!(initial_shared_version.value(), 3);
                assert!(!mutable);
            }
            other => panic!("unexpected argument: {other:?}"),
        }
        match to_legacy_object_arg(&value, None, Some(SharedAccess::Mutable)).unwrap() {
            ObjectArg::SharedObject {
                initial_shared_version,
                mutable,
                ..
            } => {
                assert_eq!(initial_shared_version.value(), 3);
                assert!(mutable);
            }
            other => panic!("unexpected argument: {other:?}"),
        }
    }

    #[test]
    fn unsupported_owner_kinds_fail_closed() {
        assert!(
            to_legacy_object_arg(
                &record(NeutralOwner::Object(ADDRESS.to_owned())),
                None,
                None
            )
            .is_err()
        );
        assert!(
            to_legacy_object_arg(
                &record(NeutralOwner::ConsensusV2 {
                    address: ADDRESS.to_owned(),
                    start_version: 2
                }),
                None,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn invalid_version_and_shared_version_fail_closed() {
        let mut value = record(NeutralOwner::Immutable);
        value.version = 0;
        assert!(to_legacy_object_arg(&value, None, None).is_err());
        let shared = record(NeutralOwner::Shared {
            initial_shared_version: 0,
        });
        assert!(to_legacy_object_arg(&shared, None, Some(SharedAccess::ReadOnly)).is_err());
    }
}
