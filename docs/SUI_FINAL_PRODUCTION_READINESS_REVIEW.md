# Final Sui production-readiness review

Review date: 2026-10-30

Reviewed branch: `fix/sui-production-inventory-contract`

Reviewed code:
- `src/library/get_coin_object_ids_by_amount.rs`
- `src/utils/sui.rs`
- `src/features/swap/build_tx.rs`
- `src/client.rs`
- `Cargo.toml`

## Verdict

The pagination validation and selection helpers are materially hardened, and the expanded CI run passed. The inventory path is **not production-ready** yet. Two integration defects remain: the inventory function creates and uses a different network client from the transaction builder, and the 30-second deadline excludes that client construction. SUI's production gas-coin path also does not use the coin objects selected by the inventory check.

A gRPC migration is necessary for a Sui Foundation Mainnet integration as of this review date. It should be treated as a deliberate SDK/API migration, not a drop-in transport toggle.

## 1. Deadline and client lifecycle — BLOCKER

The 30-second deadline is created only after `get_sui_client().await` returns. Therefore it does not bound client construction.

More importantly, `src/client.rs` currently constructs localnet, testnet, and devnet clients sequentially, then returns the localnet client. The inventory collector calls this helper internally. By contrast, `build_tx` already receives a `&SuiClient` and uses that supplied client for the transaction's other reads.

This means coin inventory can be read from `http://127.0.0.1:9000` while the transaction is being built against the caller's different network client. Coin object IDs selected from one network are not valid evidence for the other network. This is a production blocker.

Required correction:
- Pass the caller's existing client into the inventory collector and through `Ptb::get_split_coin_for_tx`.
- Remove internal client construction from the inventory function.
- Start the single absolute deadline immediately before the first inventory RPC.
- Keep the remaining-time timeout around every page request.

## 2. RPC failures and timeout — PARTIALLY VERIFIED

The production async path fails closed:
- RPC errors are returned with the page number in context.
- The remaining absolute deadline is applied to each page request.
- Timeout and RPC errors do not return a partial successful inventory.
- There is no retry that could silently extend the absolute deadline.

The current tests exercise pure pagination validation and selection logic. They do not inject an RPC failure or a deliberately stalled RPC future, so the timeout/cancellation behavior is compile-checked but not yet behaviorally tested.

Required correction:
- Add a mockable page-fetch boundary or equivalent deterministic tests for first-page RPC failure, later-page RPC failure, and a stalled request that exceeds the absolute deadline.
- Confirm that a failure after one or more successful pages returns an error and exposes no partial coin selection.

## 3. Pagination bounds — GUARDED, NOT CALIBRATED

Current safeguards:
- Requested page size: 1,000.
- Maximum pages: 100.
- Maximum total coin objects: 10,000.
- Repeated cursors, missing cursors when another page is indicated, oversized pages, duplicate object IDs, object-count overflow, and balance overflow are rejected.
- An empty intermediate page continues when a next cursor is supplied.
- A final page is accepted only when the RPC reports no next page.

These are fail-closed safety ceilings, not measured production maxima. The 10,000-object cap can reject a legitimate account with a larger inventory. The current tests cover the pure bounds and cursor rules, but not an end-to-end paginated RPC sequence. Snapshot consistency across pages and independent-provider reconciliation are not established.

Required correction:
- Preserve the caps until account-distribution measurements justify changing them.
- Add deterministic multi-page fetch tests that prove the cursor is passed unchanged, the natural end is reached, and no partial inventory is returned after a later-page failure.
- Treat snapshot consistency as an explicit unresolved property unless the selected RPC/API provides a documented checkpoint-scoped snapshot.

## 4. SUI gas-coin behavior — BLOCKER FOR SWAP READINESS

`Ptb::get_split_coin_for_tx` always calls the inventory collector first. For SUI:
- When `inspect_transaction` is true, the builder merges and splits the selected owned coin objects.
- When `inspect_transaction` is false, it ignores those selected objects and splits `Argument::GasCoin`.

The gas-coin path therefore checks aggregate SUI coin-object balance but does not prove that the gas coin selected for the final transaction can cover the swap input plus gas budget. The simulation path and final-build path also use different SUI input construction.

Required correction:
- Define one explicit SUI funding contract for both simulation and final transaction construction.
- Ensure the selected gas payment or address-balance funding mechanism is sufficient for swap input and gas.
- Make simulation exercise the same funding shape as the final transaction.
- Do not describe aggregate coin inventory as proof that the eventual gas coin is sufficient.

## 5. Is gRPC migration necessary? — YES FOR SUI FOUNDATION MAINNET

The official Sui documentation states that JSON-RPC was disabled on Sui Foundation Mainnet full nodes the week of July 27, 2026, with full decommission planned for mid-October 2026. This review is dated October 30, 2026.

The repository currently pins the legacy `sui_sdk` crate to Sui tag `testnet-v1.39.3` and uses its JSON-RPC `coin_read_api().get_coins` interface. Current official Rust guidance identifies `sui-rust-sdk` as the actively maintained Rust SDK using gRPC and GraphQL; the legacy `sui-sdk` crate no longer includes the JSON-RPC client.

Decision:
- For a production Sui Foundation Mainnet target, migrate to the current Rust gRPC client (or explicitly choose GraphQL where its indexed-query properties are required).
- Prefer gRPC for this coin-inventory and transaction-simulation path.
- The prior bounded gRPC pagination proof is useful evidence for pagination mechanics, but it is not an implementation of this repository's production adapter. API mapping, transaction construction, simulation, and network selection still require dedicated migration work.
- Do not assume the old JSON-RPC client remains viable against Foundation Mainnet. A third-party provider that still serves JSON-RPC would be a temporary provider-specific exception, not the preferred long-term architecture.

## Recommended implementation order

1. Fix the network-client mismatch by passing the transaction builder's client into inventory reads.
2. Unify SUI gas/input behavior between simulation and final build.
3. Add deterministic RPC failure and deadline tests plus multi-page integration tests.
4. Create a separate isolated gRPC migration branch using the current Rust SDK; port inventory and simulation deliberately.
5. Run CI and bounded read-only network verification against the intended Sui network/provider.
6. Keep `main` untouched until the full migration and transaction-builder behavior are verified.

## Scope and safety

This review made no transaction submissions, signatures, broadcasts, or network writes. It does not authorize merging or changing `main`.


## Follow-up implementation update — 2026-10-30

The two immediate integration corrections have now been implemented on this isolated branch:

1. `get_coin_object_ids_by_amount` now receives the caller's `&SuiClient`; `Ptb::get_split_coin_for_tx` passes that same client through. The inventory function no longer constructs its own client, eliminating the localnet-vs-caller-network mismatch. Its absolute 30-second deadline begins immediately before the first inventory RPC.
2. Native SUI input now uses `Argument::GasCoin` for both dev-inspect and ordinary transaction construction. Route split totals are checked for overflow and must equal the requested SUI amount. The old branch that used selected owned coin objects only during inspection has been removed.
3. Added unit tests for SUI split-total match, mismatch, and overflow.

Important remaining boundary: using `GasCoin` consistently makes the transaction shape consistent, but does not prove that the wallet's eventual gas-payment object has enough balance for the SUI swap input plus gas. The wallet-selected gas payment must be exercised in dev-inspect and final transaction assembly. The RPC failure/deadline tests are also still not mocked at this point.

The implementation is pending CI verification. No gRPC migration has started; it will be a separate branch after this correction passes tests. `main` remains untouched.
