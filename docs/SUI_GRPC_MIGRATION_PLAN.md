# Sui gRPC Migration — Architecture Review and Staged Plan

Status: architecture review only; no runtime dependency or transaction code changed in this commit.

Baseline: `3e7ac7a62ee631a0b04500c1897a85b9b66a178d` on `fix/sui-production-inventory-contract`.
This migration branch was created from that exact commit. Keep `main` and the source branch unchanged.

## Findings

1. The current application pins the legacy `sui-sdk`, `sui-keys`, `move-core-types`, `shared-crypto`, and `sui-transaction-builder` crates to the old `testnet-v1.39.3` tag.
2. Runtime code uses legacy `SuiClient` across inventory reads, object lookups, dev-inspect, route execution construction, and gas estimation. Transaction construction uses legacy `ProgrammableTransactionBuilder`, `Argument`, `ObjectArg`, `ObjectRef`, `TypeTag`, and `TransactionKind`.
3. Mysten's maintained `sui-rust-sdk` is modular. Its current `sui-rpc` crate is version `0.4.0`, exposes `sui_rpc::Client`, gRPC protobuf request/response types, and `sui_sdk_types` core types. This is not a drop-in replacement for the old client/types.
4. The gRPC `StateService.ListOwnedObjects` request supports owner, page size, page token, read mask, and object-type filtering. For coin inventory, the intended filter is the exact `0x2::coin::Coin<T>` type and the object read mask must include object ID, version, digest, object type, owner, and Move contents. Coin balances must be decoded from BCS contents with strict type/length validation; aggregate `ListBalances` data alone is not enough to choose concrete coin objects for a transaction.
5. The gRPC client provides a paginated `list_owned_objects` stream, but the application must retain its own fail-closed object/page/deadline bounds and verify natural termination. Stream errors must not be converted into successful partial inventory.
6. Existing transaction PTB code is coupled to legacy Sui transaction types throughout the protocol adapters. A mass import replacement or dependency-only change is unsafe.

## Staged migration sequence

### Stage 0 — preserve and map (this review)
- Preserve the passing inventory/gas implementation and all existing proof branches.
- Map legacy client/type dependencies and document the gRPC target surface.
- Do not change Cargo dependencies, transaction types, endpoint defaults, or signing behavior.

### Stage 1 — gRPC read-only inventory adapter
- Add a narrowly scoped gRPC client constructor with explicit network/endpoint configuration; no implicit localnet fallback.
- Implement a read-only adapter for listing owned `Coin<T>` objects using `ListOwnedObjects`, page tokens, a required read mask, and strict BCS decoding.
- Preserve the current 30-second absolute deadline, page/object limits, duplicate-ID and cursor checks, checked arithmetic, and insufficient-inventory failure.
- Keep the existing legacy transaction builder untouched in this stage. Do not claim this stage makes transaction execution production-ready.
- Add deterministic mock gRPC tests for multiple pages, empty pages with continuation tokens, malformed/missing contents, wrong coin type, duplicate IDs, repeated tokens, timeout, RPC error, and cap exhaustion.

### Stage 2 — read-only object and network surface
- Migrate object lookups used by protocol adapters to gRPC object reads.
- Convert returned object IDs, versions, digests, owner and shared-object metadata into an explicit internal domain model.
- Verify shared-object initial versions and mutability semantics; do not assume legacy `OBJECT_START_VERSION` is a valid replacement.
- Keep this stage read-only and test all protocol object requirements.

### Stage 3 — simulation and gas semantics
- Identify the supported gRPC simulation/dev-inspect method and its transaction request format.
- Migrate simulation and gas estimation only after the new transaction representation exists.
- Preserve fail-closed status handling and checked gas arithmetic.
- Verify the exact selected gas payment object and that its balance covers input plus gas; aggregate wallet balance is insufficient evidence.

### Stage 4 — transaction builder and protocol adapters
- Migrate the PTB abstraction to the maintained transaction-builder and `sui-sdk-types` types.
- Migrate all DEX protocol adapters against the new `Argument`/transaction model as one controlled compatibility effort.
- Add serialization/BCS equivalence tests for representative Move calls, type arguments, object arguments, split/merge, transfer and shared-object usage.
- Keep transaction signing and broadcast disabled during migration tests.

### Stage 5 — end-to-end readiness and legacy removal
- Run CI across formatting, compile-all-targets, unit tests, mock gRPC integration tests and read-only live testnet smoke tests.
- Reconcile simulation inputs, final transaction inputs, gas payment and object references.
- Remove legacy dependencies only when the repository has no remaining legacy imports and all required transaction behavior is verified.
- Do not merge to `main` or create a PR without separate explicit authorization.

## Important repository evidence correction

The current `Cargo.toml` on the existing pagination-hardening branch still pins the old `sui-sdk` tag. Therefore that branch's passing pagination tests demonstrate pagination-contract behavior, not an integrated Mysten `sui-rpc` gRPC production adapter. The gRPC adapter must be built and tested independently here.

## Exit criteria for Stage 1

- The gRPC client is constructed for an explicitly selected network and endpoint.
- Every inventory page is consumed until the server returns no next-page token, within strict application limits.
- Coin object contents are decoded and validated without panics or lossy defaults.
- Every error, timeout, malformed response, repeated token, duplicate ID or limit breach fails closed.
- Tests prove that no partial inventory is returned as success.
- No signing, transaction submission, or writes occur.

## Stage 1 implementation result

- Implemented as an isolated library crate at `crates/sui-grpc-inventory`, using Mysten `sui-rpc = 0.4.0`.
- The adapter builds a client only from an explicit HTTPS endpoint, requests `Coin<T>` objects with an explicit field mask, validates owner/type/object ID/version/digest, and strictly decodes the BCS `(ID, balance)` layout.
- Pagination remains fail-closed with a 30-second absolute deadline, page size 1,000, 100-page cap, 10,000-object cap, empty-page continuation, duplicate ID/token detection, checked balance arithmetic, and insufficient-inventory errors.
- Deterministic mock page-source tests cover multiple pages, empty intermediate pages, RPC errors, deadline expiry, malformed/missing BCS, wrong type/owner, duplicate IDs, repeated/empty tokens, page/object caps, insufficient balance, and balance overflow.
- CI run [41094471205](https://github.com/navedjuwale-creator/7k-sdk-rs/actions/runs/41094471205) passed formatting, `cargo check --all-targets`, and all 15 tests.
- The adapter is intentionally not yet wired into the legacy application crate. A direct root dependency on `sui-rpc` exposed an upstream dependency incompatibility between the old pinned Sui SDK's `axum-server` and the newer resolved `hyper-util`; keeping Stage 1 standalone avoids changing the legacy transaction stack. The first integration attempt and its compile failure were removed from the root dependency graph.
- These tests mock the page-source boundary; they do not yet prove live endpoint connectivity or a wire-level mock gRPC server. No live RPC call, signing, transaction construction, or broadcast was performed.

Stage 1 exit criteria are met for the standalone read-only adapter and its deterministic mock tests. Stage 2 remains gated on separate authorization to continue the migration plan; do not change the transaction builder as part of this Stage 1 result.
