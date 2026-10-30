# Sui SDK Security Remediation Assessment

**Status:** Investigation only — no dependency pins changed.

## Verified repository facts

- `Cargo.toml` pins five packages from `https://github.com/mystenlabs/sui` to `testnet-v1.39.3`: `sui-sdk`, `sui-keys`, `move-core-types`, `shared-crypto`, and `sui-transaction-builder`.
- Source code uses the legacy `sui_sdk::SuiClient` / `SuiClientBuilder` API, `coin_read_api().get_coins(...)`, legacy transaction `Argument` / `TypeTag`, and `move_core_types::language_storage::StructTag`.
- The repository root and its `src` tree do not contain `sui/boat_fun/Move.toml`; there is no root `.gitmodules` file. The Cargo.toml comments referring to that path cannot be verified from this repository. A separately maintained Move package may exist elsewhere, but it is not included here.
- The dependency audit still fails after a lockfile-only refresh. The diagnostic workflow traces reverse dependency paths.

## Compatibility finding

A blind change from `testnet-v1.39.3` to a recent Sui release is not a safe remediation. Sui's current documentation labels `sui-sdk` in the main Sui repository as the **legacy** Rust SDK and identifies `sui-rust-sdk` as the current SDK for new integrations. The current SDK is modular and uses the newer RPC surface. This repository directly depends on legacy client, coin-read, transaction-building, and Move type APIs.

Therefore, simply advancing the old Git tag may leave APIs unavailable or materially changed. Moving to `sui-rust-sdk` requires a planned source migration, not a lockfile-only update.

Reference: https://docs.sui.io/references/rust-sdk

## Safe remediation sequence

1. Preserve the current pinned dependency graph as the reproducible baseline.
2. Obtain the authoritative Move package repository/path and its intended Sui network/release compatibility. Do not assume the missing `sui/boat_fun/Move.toml` is present in this repository.
3. Inventory all direct and transitive uses of the legacy Sui crates.
4. Create a separate migration branch from `main` for the current `sui-rust-sdk` API. Port client creation, coin reads, transaction construction, signing/key handling, and Move type parsing independently, with regression tests.
5. In parallel, address non-Sui direct dependencies (for example `anyhow`) only where a compatible, tested update exists; do not force transitive versions or suppress advisories.
6. Run formatting, `cargo check`, all tests, and `cargo audit` on the isolated branch. Compare behavior and transaction serialization before proposing a PR.

## Safety boundary

No source dependency pins were changed by this assessment. No signing, transaction submission, release, merge, or changes to `main` were performed. This branch starts from the current `main` baseline and contains only this assessment document.
