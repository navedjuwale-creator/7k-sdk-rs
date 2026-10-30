# Sui production coin inventory contract

This contract applies to `get_coin_object_ids_by_amount` and the transaction-building
path that consumes its selected coin objects. The caller must pass the same `SuiClient`
used for transaction construction; the inventory collector must never create its own client.

## Success conditions

A successful result means all pages for the requested owner and exact coin type
were retrieved, the RPC reported a natural end, the complete inventory balance
was accumulated without overflow, and the selected coins cover the requested
amount. A partial inventory is never returned as success.

## Bounded collection

The current implementation uses these initial production safeguards:

- Page size: 1,000 objects.
- Maximum pages: 100.
- Maximum total objects: 10,000.
- One absolute 30-second deadline for the complete pagination operation.

These are safety ceilings, not evidence that an account has fewer objects. If a
ceiling is reached before natural pagination completion, the operation returns
an explicit incomplete-inventory error. It must not reinterpret the partial
balance as an insufficient-balance result.

The bounds should be revisited with measured production account distributions
before being raised or made configurable. In particular, the 10,000-object
ceiling may reject legitimate large inventories by design; this is safer than
silently selecting from an incomplete set.

## Pagination and integrity

- An empty page does not terminate pagination if the server says another page exists.
- A next-page indication without a cursor is an error.
- Repeated cursors are an error.
- Duplicate coin object IDs across or within pages are an error.
- RPC errors, deadline expiry, oversized pages, object-count overflow, and
  balance overflow are errors.
- A cursor is only followed after validation.
- The complete inventory is collected before coin selection begins.

## Selection

- A zero requested amount is rejected.
- Coins are ordered by descending balance.
- Selection stops once the requested amount is covered.
- Checked addition prevents arithmetic wraparound.
- A complete but insufficient inventory returns an explicit insufficient-balance
  error; no coin IDs are returned to transaction construction.

## Remaining verification boundary

This change does not prove snapshot consistency across pages, server indexing
completeness, or agreement with an independent RPC provider. The pagination
operation is read-only. No signing, broadcasting, or transaction submission is
performed by this inventory function.

Unit tests cover empty intermediate/final pages, missing and repeated cursors, page/object bounds, oversized pages, duplicate object IDs, zero requests, insufficient inventory, and checked selection arithmetic. RPC failure and deadline handling remain enforced in the production async path and are compile-checked, but are not yet exercised through a mocked RPC integration test.

CI must compile the production crate and run its unit tests before this branch
can be considered verified.


Additional regression tests verify that SUI route split amounts equal the requested amount
and reject mismatch and arithmetic overflow. These tests do not establish that an eventual
wallet-selected gas payment object has sufficient balance; that must be verified by the
same transaction simulation and the wallet's final gas-payment selection.
