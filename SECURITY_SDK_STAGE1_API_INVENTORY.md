# Sui Rust SDK Migration — Stage 1 API Inventory

**Branch:** `security/sui-sdk-migration-assessment`  
**Baseline:** `main` at `0b2cb6ddbe745977454c02a2b13b2c8e3ad98df1`  
**Status:** Inventory complete; implementation intentionally not started.

## Why a version-only update is unsafe

The project pins five packages from the monolithic `MystenLabs/sui` repository at `testnet-v1.39.3`:

- `sui-sdk`
- `sui-keys`
- `move-core-types`
- `shared-crypto`
- `sui-transaction-builder`

The official Sui documentation now identifies `sui-rust-sdk` as the current SDK and the monorepo `sui-sdk` crate as legacy. The legacy crate no longer provides the former JSON-RPC client. The current SDK is split into crates including `sui-rpc`, `sui-sdk-types`, `sui-crypto`, and `sui-transaction-builder`.

Official references:
- https://docs.sui.io/references/rust-sdk
- https://docs.sui.io/references/sui-sdks
- https://github.com/MystenLabs/sui-rust-sdk

## Confirmed migration surface

### Network and reads

- `src/client.rs`: `SuiClientBuilder`, `.build("http://127.0.0.1:9000")`, `.build_testnet()`, `.build_devnet()`, and `api_version()`.
- `src/library/get_coin_object_ids_by_amount.rs`: `coin_read_api().get_coins(...)`, `Coin`, `ObjectID`, and `SuiAddress`.
- `src/utils/sui.rs`: `read_api().get_owned_objects(...)`, `get_object_with_options(...)`, `SuiObjectResponse`, `SuiObjectDataOptions`, `SuiObjectResponseQuery`, and `ObjectRef`.

### Transaction construction and Move types

- `src/utils/sui.rs`: `ProgrammableTransactionBuilder`, `Argument`, `Command`, `ObjectArg`, `TransactionKind`, `TypeTag`, `TypeInput`, `ObjectRef`, `Identifier`, `ObjectID`, and Sui clock constants.
- `src/library/swap_with_route.rs`: `SuiClient`, transaction `Argument`, and `TypeTag`.
- `src/features/swap/build_tx.rs`: `SuiClient`, `SuiAddress`, and transaction `Argument`.
- Protocol adapters including `src/library/protocols/deepbook.rs` and `src/library/protocols/cetus.rs` use legacy transaction arguments and addresses.

The above is a confirmed minimum migration surface, not an exhaustive symbol inventory. More adapter files must be compiled and searched during the port.

## Important network compatibility issue

Sui documentation states JSON-RPC was disabled on Sui Foundation Mainnet in the week of July 27, 2026, with full decommission planned for mid-October 2026. Existing integrations must move to gRPC or GraphQL. The current Rust SDK supports gRPC and GraphQL, not JSON-RPC.

This makes migration necessary for mainnet compatibility; merely raising the old Sui Git tag is not a sufficient solution.

## Stage 1 implementation decision

Do **not** replace the five old dependencies with guessed crate versions or change the lockfile blindly. The current SDK workspace manifest identifies `sui-rpc 0.4.0`, `sui-sdk-types 0.4.0`, and `sui-transaction-builder` as modular crates, but exact crate APIs and the compatible dependency graph must be verified against the selected released crate versions before editing this application.

The first code change should be a narrow read-client adapter behind the existing project boundary. It should provide:
1. Explicit configured endpoint and transport (gRPC or GraphQL), not implicit localnet/testnet/devnet construction.
2. Read-only coin and owned-object queries with explicit pagination.
3. Typed errors for unsupported endpoints and missing data.
4. Tests using fixtures/mocks; no signing or transaction execution in this stage.

Only after that adapter compiles and passes tests should the transaction-builder and protocol adapters be migrated.

## Security and test requirements

- Keep all changes on this branch, based on `main`.
- Do not suppress RustSec advisories or use `[patch]` overrides to force arbitrary transitive versions.
- Check `cargo audit` against the exact lockfile after each dependency change.
- Run `cargo fmt --check`, `cargo check --all-targets`, and `cargo test --all-targets`.
- Add regression tests for pagination, empty coin lists, insufficient balances, and overflow-safe balance accumulation before considering the migration complete.
- Do not merge, release, sign, or submit transactions as part of this work.

## Result

Stage 1 has established that this is a cross-cutting API and transport migration, not a routine dependency bump. No application source, dependency pins, or lockfile were changed in this stage.
