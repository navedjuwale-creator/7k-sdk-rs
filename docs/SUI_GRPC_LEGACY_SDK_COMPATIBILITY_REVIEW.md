# Sui gRPC / legacy SDK compatibility investigation

Date: 2026-10-30
Branch: `migrate/sui-grpc-client`
Scope: read-only source and dependency inspection; no transaction construction, signing, broadcast, or chain writes.

## 1. Exact legacy SDK baseline

The root `Cargo.toml` pins the MystenLabs Sui repository at tag `testnet-v1.39.3` for `sui-sdk`, `sui-keys`, `move-core-types`, `shared-crypto`, and `sui-transaction-builder`. The root `Cargo.lock` resolves `sui-sdk` version `1.39.3`, with the matching Sui types and transaction-builder packages.

Inspected upstream source at that exact tag:
- `crates/sui-types/src/transaction.rs`
- `crates/sui-types/src/object.rs`
- `crates/sui-types/src/base_types.rs`
- `crates/sui-types/src/digests.rs`
- `crates/sui-types/src/programmable_transaction_builder.rs`

## 2. Legacy type capabilities established from source

### Transaction arguments

At `testnet-v1.39.3`, `ObjectArg` has exactly these variants:
- `ImmOrOwnedObject(ObjectRef)`
- `SharedObject { id, initial_shared_version, mutable }`
- `Receiving(ObjectRef)`

`ObjectRef` is the tuple `(ObjectID, SequenceNumber, ObjectDigest)`. The source provides parsers for object IDs, Sui addresses, and base58 object digests, plus `SequenceNumber::from_u64`.

### Owner representations

The legacy `Owner` enum distinguishes:
- `AddressOwner(SuiAddress)`
- `ObjectOwner(SuiAddress)`
- `Shared { initial_shared_version }`
- `Immutable`
- `ConsensusV2 { start_version, authenticator }`

### Conversion compatibility matrix

| Neutral gRPC owner | Legacy target | Result | Required guard |
|---|---|---|---|
| Address | `ImmOrOwnedObject(ObjectRef)` | Supported conditionally | Exact owner address must equal the explicitly supplied transaction sender; parse and preserve the exact object ID, version, and digest. |
| Immutable | `ImmOrOwnedObject(ObjectRef)` | Supported conditionally | Preserve exact object ID, version, and digest; never infer immutability from missing owner data. |
| Shared | `SharedObject` | Supported conditionally | Preserve the exact nonzero initial shared version; mutability must be an explicit caller input, never inferred. |
| Object-owned | `Receiving(ObjectRef)` | Not approved in this step | The enum variant exists, but safe use requires separately proving Receiving/parent-object semantics and the required parent input. Do not coerce to address-owned. |
| Consensus-address / ConsensusV2 | No lossless `ObjectArg` mapping established | Unsupported | The neutral record carries address and start version, but the legacy `ObjectArg::SharedObject` has no authenticator field. Do not coerce it to Shared. |

The legacy `ProgrammableTransactionBuilder::obj` merges repeated shared references only when their initial shared versions match, and combines mutability with logical OR. The adapter must therefore preserve the initial version and avoid hiding mutability escalation when callers add the same object more than once.

## 3. Existing adapter boundary

The gRPC adapter remains a separate crate, `crates/sui-grpc-inventory`, with `sui-rpc = 0.4.0`. It exposes neutral `SuiObjectRecord` / `SuiOwner` types and does not currently depend on `sui-sdk`.

The root lockfile contains the legacy SDK graph (including `axum-server` and `hyper-util`) but no `sui-rpc` package. The adapter has its own manifest. A previous root-level attempt to combine the gRPC dependency with the legacy SDK encountered an upstream dependency-resolution conflict involving `axum-server` / `hyper-util`; this is a recorded prior integration result, not a fresh claim that every isolated arrangement is impossible.

**Not yet proven:** whether one isolated crate can compile against both `sui-rpc = 0.4.0` and the exact `sui-sdk` 1.39.3 dependency graph without conflict. No combined-dependency compile probe has been run in this investigation.

## 4. Decision

A narrow pure conversion boundary is source-feasible for:
1. address-owned objects, with exact sender equality;
2. immutable objects, with exact reference preservation;
3. shared objects, with exact initial shared version and explicit read-only/mutable intent.

Object-owned and ConsensusV2 records remain rejected. The existence of `ObjectArg::Receiving` is not sufficient evidence to enable object-owned conversion.

However, do not add the conversion module to the live gRPC adapter until a dedicated CI compatibility probe proves that the adapter can compile with both dependency families. This is the next gate. The probe must not create or build a transaction; it should only compile typed conversion code and run deterministic unit tests.

## 5. Safety boundary

This investigation changed no source code and no dependency manifests. No transaction was constructed, simulated, signed, or broadcast. No on-chain write was attempted. No PR or merge was created. Production `main` remains untouched.
