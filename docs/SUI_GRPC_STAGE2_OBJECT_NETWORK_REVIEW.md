# Stage 2 — Read-only Object and Network Compatibility Review

Status: review complete; implementation intentionally not started. Stage 1 files and tests are unchanged.

Reviewed branch: `migrate/sui-grpc-client`
Reviewed HEAD before this document: `c9497cd384f5b63e74af56da7c735d6b01c7fe75`
Production `main` baseline remains `0b2cb6ddbe745977454c02a2b13b2c8e3ad98df1`.

## Executive result

Stage 2 is **not ready for transaction-facing integration**. The gRPC read-only object surface is available, but the existing application currently loses essential owner metadata and chooses the wrong network client. The safe next implementation is a read-only object metadata adapter plus explicit network identity verification. It must not convert metadata into legacy transaction arguments yet.

## Findings

### S2-01 — Critical: network selection currently returns localnet unconditionally

`src/client.rs::get_sui_client` constructs localnet, testnet, and devnet clients, prints testnet/devnet API versions, then returns `sui_localnet`. Its own TODO says the SDK only works on mainnet, which conflicts with the returned localnet client.

Consequences:
- Callers cannot reliably select mainnet/testnet/devnet.
- Unrelated network client construction occurs as a side effect.
- The returned client can target localhost even when the caller expects a public network.
- Stage 2 cannot safely bind object metadata to a network until this is replaced with explicit configuration.

Required correction in the Stage 2 implementation:
- Require a caller-supplied network enum and endpoint; no implicit localnet/default.
- Construct exactly one gRPC client.
- Call gRPC `LedgerService.GetServiceInfo` and require a non-empty `chain_id`.
- Compare the observed chain ID to the configured expected chain ID; fail closed on mismatch or missing identity.
- Treat the human-readable chain name as diagnostic only; the genesis-checkpoint chain ID is the identity key.
- Do not persist or cache object references across different chain IDs.

### S2-02 — High: the gRPC object read method is LedgerService.GetObject, not StateService

The maintained Sui API definitions place `GetObject` and `BatchGetObjects` on `LedgerService`; `ListOwnedObjects` is on `StateService`. Stage 1 uses the latter correctly for inventory. Stage 2 should use the ledger client for point reads and request a deliberate field mask:
- `object_id`
- `version`
- `digest`
- `owner`
- `object_type`
- `contents` only for callers that need BCS Move contents

Every required field must be checked for presence. Do not silently substitute defaults. A missing object, deleted object, unsupported response shape, or malformed owner must be an error.

### S2-03 — Critical: shared-object initial version is currently discarded

In `src/utils/sui.rs`, `ObjectRefFetcher::object_ref` reads a legacy `ObjectRef`, which contains ID/version/digest but not owner metadata. The helper `ObjectArgExt::shared_obj` then assigns `OBJECT_START_VERSION` to every shared object.

That is not a valid general conversion. The gRPC owner model explicitly distinguishes:
- Address owner (kind 1)
- Object owner (kind 2)
- Shared owner (kind 3), with `version` carrying the initial shared version
- Immutable (kind 4)
- Consensus-address owner (kind 5), with `version` carrying the start version and `address` the owner

Stage 2 must preserve this data in an internal read-only domain model. For a shared object, the exact `Owner.version` must be retained as `initial_shared_version`; do not replace it with a constant. For consensus-address ownership, preserve its distinct kind and start version. Do not coerce it to ordinary address-owned or shared metadata.

### S2-04 — High: shared mutability is caller intent, not a property inferred from a read

The current helpers expose `shared_obj_mut` and `shared_obj_imut`, but a read-only object record should report ownership and initial shared version without deciding how a future transaction uses it. The future transaction layer must request mutability explicitly and validate that the object is actually shared. Immutable, address-owned, object-owned, and consensus-address objects must not be silently converted into a shared-object argument.

### S2-05 — High: legacy object-reference types are not a sufficient Stage 2 boundary

The existing PTB code uses legacy `ObjectRef`, `ObjectArg`, `ObjectID`, `SuiAddress`, and `Argument`. The maintained `sui-sdk-types` model has explicit `Owner` variants and an `ObjectReference` of ID/version/digest. Stage 2 should introduce a neutral read-only record rather than importing either SDK's transaction types into the adapter.

Proposed read-only record fields:
- verified `chain_id`
- canonical object ID
- object version
- object digest
- parsed object type
- owner kind
- owner address/object ID where applicable
- initial shared version or consensus start version where applicable
- optional raw BCS contents (only when explicitly requested)

No conversion to `ObjectArg` or transaction builder types in Stage 2.

### S2-06 — High: JSON-RPC endpoint assumptions are obsolete for current Sui Foundation fullnodes

The repository pins the legacy Sui SDK to `testnet-v1.39.3` and calls its legacy read APIs. Current Sui documentation identifies `sui-rust-sdk` as the current SDK and states that JSON-RPC was disabled on Sui Foundation mainnet fullnodes from the week of July 27, 2026, with broader decommission planned for mid-October 2026. Provider-specific endpoint behavior may differ, but the existing legacy client cannot be treated as a durable network abstraction.

This reinforces the need for explicit gRPC endpoint and chain identity configuration. It does not justify a broad dependency migration in this Stage 2 step.

## Stage 2 implementation acceptance criteria

1. Add a read-only gRPC object adapter using `LedgerService.GetObject` with explicit field masks.
2. Add a network configuration type requiring endpoint plus expected chain ID.
3. Verify the endpoint's reported chain ID through `GetServiceInfo` before serving object reads.
4. Preserve owner variants and exact shared/consensus versions in a neutral domain record.
5. Reject missing ID/version/digest/type/owner fields and malformed owner combinations.
6. Test address-owned, object-owned, shared, immutable, and consensus-address objects.
7. Test chain-ID match, mismatch, missing chain ID, malformed service info, object-not-found, missing object fields, and RPC errors.
8. Keep this crate read-only; no PTB conversion, transaction building, simulation, signing, broadcast, or writes.
9. Preserve Stage 1 source and its passing CI baseline; add a separate Stage 2 workflow and tests.
10. Do not merge or create a PR without explicit authorization.

## Evidence references

- Sui gRPC `StateService.ListOwnedObjects` and field-mask contract: https://github.com/MystenLabs/sui-apis/blob/main/proto/sui/rpc/v2/state_service.proto
- Sui gRPC `LedgerService.GetObject`, `GetServiceInfo`, and object response: https://github.com/MystenLabs/sui-apis/blob/main/proto/sui/rpc/v2/ledger_service.proto
- Sui object fields: https://github.com/MystenLabs/sui-apis/blob/main/proto/sui/rpc/v2/object.proto
- Sui owner kinds and version semantics: https://github.com/MystenLabs/sui-rust-sdk/blob/master/crates/sui-rpc/vendored/proto/sui/rpc/v2/owner.proto
- Maintained Rust SDK object owner model: https://github.com/MystenLabs/sui-rust-sdk/blob/master/crates/sui-sdk-types/src/object.rs
- Sui Rust SDK status and JSON-RPC migration guidance: https://docs.sui.io/references/sdks/rust and https://docs.sui.io/references/sui-sdks

## Recommendation

Proceed with a narrowly scoped Stage 2 read-only adapter and network identity handshake, starting with tests for all five owner kinds and chain-ID mismatch. Do not touch the existing legacy PTB layer until this boundary is proven.
