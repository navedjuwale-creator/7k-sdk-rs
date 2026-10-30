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

### Combined dependency CI probe — failed

A dedicated isolated probe was run on branch `test/sui-grpc-legacy-sdk-compat`, commit `df820cd5c3e1b3ecbc7e96bf4d97227852214b3d`.

Workflow: [CI run 41106365598](https://github.com/navedjuwale-creator/7k-sdk-rs/actions/runs/41106365598)

The probe included both `sui-rpc = 0.4.0` and the exact `sui-sdk` / `sui_types` tag `testnet-v1.39.3`. It reused the repository's root lockfile to avoid the unrelated yanked-`core2` resolution failure encountered in the first attempt.

Dependency resolution proceeded, but compilation failed inside the upstream pinned `axum-server` dependency. Rust reported that the associated `BodyData` type did not satisfy `bytes::Buf` at calls to `hyper_util::server::conn::auto::Builder::serve_connection_with_upgrades` (`axum-server` source `server.rs`, around lines 202 and 205). The failing package is upstream `axum-server v0.6.1`, pulled from the Sui tag's pinned git revision. The probe did not reach compilation of the pure conversion module or its unit tests.

**Interpretation:** the combined dependency graph is not currently buildable in this pinned configuration under the CI toolchain. This is concrete evidence of an upstream dependency/toolchain incompatibility, not a failed conversion assertion and not proof that the conversion logic itself is wrong. Do not work around it by arbitrarily upgrading `hyper-util`, patching upstream crates, or changing the production SDK pins as part of this recovery step.

## 4. Decision

A narrow pure conversion boundary is source-feasible for:
1. address-owned objects, with exact sender equality;
2. immutable objects, with exact reference preservation;
3. shared objects, with exact initial shared version and explicit read-only/mutable intent.

Object-owned and ConsensusV2 records remain rejected. The existence of `ObjectArg::Receiving` is not sufficient evidence to enable object-owned conversion.

However, the combined dependency probe failed inside upstream `axum-server` before reaching the conversion module. Do not add the conversion module to the live gRPC adapter and do not force the dependency graph with ad hoc upgrades or patches.

Recommended architecture: keep the read-only gRPC adapter and legacy transaction SDK as separate dependency domains. If typed conversion is later required, introduce a narrow, versioned neutral-record boundary (data-only object ID/version/digest/owner fields) and perform conversion in a component that depends only on the legacy SDK. Prove the handoff contract with serialization/schema tests and keep each dependency domain independently buildable. No transaction construction is part of this next design gate.

## 5. Safety boundary

The compatibility probe exists only on branch `test/sui-grpc-legacy-sdk-compat`; it does not modify the production inventory adapter or root dependency manifest. The failed run reached upstream dependency compilation but not the conversion tests. No transaction was constructed, simulated, signed, or broadcast. No on-chain write was attempted. No PR or merge was created. Production `main` remains untouched.
