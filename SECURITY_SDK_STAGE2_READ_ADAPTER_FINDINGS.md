# Sui Read-Only Adapter — Stage 2 Compatibility Findings

**Branch:** `security/sui-sdk-migration-assessment`  
**Base:** `main` at `0b2cb6ddbe745977454c02a2b13b2c8e3ad98df1`  
**Status:** API feasibility verified from published Rust crate documentation; implementation and compilation are still pending.

## Verified current Rust client surface

The published `sui-rpc 0.4.0` crate exposes `sui_rpc::Client`, a gRPC client for Sui fullnodes.

Verified documented methods include:

- `Client::new(endpoint)` — constructs a client from a URI and returns a typed status error on invalid configuration.
- `Client::select_coins(owner_address, coin_type, amount, exclude)` — selects coin objects until the requested amount is met; reports insufficient funds or RPC errors.
- `Client::list_owned_objects(request)` — returns a stream of objects and automatically follows page tokens.
- `Client::list_balances(request)` — returns a paginated stream of balances.

The new SDK separates chain types into `sui-sdk-types`, including `Address` and `TypeTag`. The transaction builder is a separate crate. This confirms that the old `SuiClient`, `Coin`, `SuiAddress`, `ObjectID`, `ObjectRef`, and PTB types cannot simply be preserved by swapping one dependency.

Published API references:
- https://docs.rs/sui-rpc/latest/sui_rpc/client/struct.Client.html
- https://docs.rs/sui-rpc/latest/sui_rpc/
- https://docs.rs/sui-sdk-types/latest/sui_sdk_types/
- https://docs.rs/sui-transaction-builder/latest/sui_transaction_builder/

## Proposed read-only adapter boundary

The smallest first adapter should own only:

1. An explicitly supplied gRPC endpoint, validated when constructing `sui_rpc::Client`.
2. A typed owner address and coin type, converted at the boundary to the current SDK types.
3. Coin selection using `Client::select_coins` for the requested minimum amount.
4. Owned-object enumeration using the stream API, collecting results with fallible stream handling.
5. Typed error propagation; no unwraps, silent empty-result substitution, or hidden fallback endpoint.

The public adapter should not expose a transaction signer, execution client, or transaction submission method. This keeps the first migration stage read-only by construction.

## Compatibility cautions

- The old `get_coins` response contained a `Coin` with `coin_object_id`, `balance`, and object-reference helpers. The new `select_coins` returns `sui_sdk_types::Object`; its object ID, type, and contents must be read from the documented object model. Do not assume field-for-field equivalence.
- `select_coins` is an amount-oriented helper, not an identical replacement for enumerating every coin. Preserve a separate all-coins path only if callers require it.
- `list_owned_objects` streams pages automatically. The adapter must propagate any stream error and avoid reporting partial results as complete.
- The old `get_owned_objects_by_page` helper has cursor and limit behavior. The new stream API is not a drop-in equivalent; API semantics need explicit mapping.
- The project's `SuiAddress` and `ObjectID` usages are widespread in protocol and PTB code. Do not change those call sites in the read-only adapter commit.

## Why source implementation is not committed in this step

This GitHub-connected workflow can inspect and commit repository files, but it cannot run this Rust project's `cargo check`, `cargo test`, or `cargo audit` on the user's build environment. A source migration without a real compiler/test result would be an unverified change, especially because the returned object model and feature requirements must be checked against the resolved crate graph.

Before source changes, a disposable CI validation workflow or an actual Rust build environment must establish:
- The exact released crate versions and compatible feature set.
- Successful compilation of a minimal client-construction and coin-read proof.
- Fixture tests for empty results, insufficient balance, stream errors, and overflow-safe total calculations.
- A clean diff that does not alter transaction construction, signing, or submission.

## Result

Stage 2 confirms that a read-only gRPC adapter is feasible with the current Rust SDK. No application source, dependency manifest, lockfile, or transaction behavior was changed. The next safe action is a minimal compile-and-test proof in isolated CI, not a speculative edit to the application.
