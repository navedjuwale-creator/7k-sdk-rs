# Owned-object pagination and completeness notes

## Scope

This note records the pagination contract verified for the isolated `sui-rpc = 0.4.0` read-only spike. It is not evidence of a successful live RPC request and does not authorize production integration.

## SDK behavior documented by upstream

The `Client::list_owned_objects` API returns a stream of `Result<Object, Status>` items. The SDK documentation states that it automatically follows `page_token` values from responses to fetch subsequent pages. An RPC failure is yielded as an error item.

References:
- https://docs.rs/sui-rpc/0.4.0/sui_rpc/client/struct.Client.html
- https://github.com/MystenLabs/sui-apis/blob/main/proto/sui/rpc/v2/state_service.proto

The request's owner is required. The optional `object_type` narrows the results. The protocol defines `next_page_token` as the signal that another page exists; an omitted token means there are no subsequent pages. When manually paginating, all other request parameters must match those used for the token.

## Completeness rule in this spike

The test-only `collect_stream` helper consumes items until the stream ends. It returns the collected vector only when the stream completes without an error. If any item is an error, it returns that error instead of exposing the accumulated prefix as a successful result.

Regression tests cover:
- collecting multiple successful items;
- treating a normally ended empty stream as an empty result;
- propagating an error after earlier successful items, rather than returning partial data.

## Important limitations

- The tests use local fixture streams; they do not prove the upstream server's pagination implementation works correctly in a live environment.
- The SDK's documented automatic pagination behavior is relied upon, not independently reimplemented by this spike.
- A clean stream end is treated as completion by the client abstraction. This test suite does not independently audit server-side indexing freshness, snapshot consistency, or omissions in the underlying data source.
- No live network request, signing, transaction construction, broadcast, or production integration is performed by this proof.

## Gate before integration

Before using this adapter for decisions involving real assets, add an explicitly approved integration test against a controlled test environment or a documented testnet endpoint, with deterministic fixtures where possible. Record endpoint identity, request filters, pagination observations, errors, and the reconciliation method. Do not interpret a successful compile or fixture test as proof of live-chain completeness.


## Local gRPC pagination fixture (SDK 0.4.0)

A deterministic local gRPC fixture now exercises the actual Client::list_owned_objects implementation, rather than a substitute stream helper. The fixture uses only synthetic object IDs and the all-zero owner filter.

The fixture scenarios are:
- **Two pages:** page one returns an object and a continuation token; page two returns a second object and no token. The test checks both objects and verifies the exact token sequence received by the mock server.
- **Empty intermediate page:** page one returns no objects but includes a continuation token; page two returns an object. The test verifies that the SDK continues rather than treating the empty page as completion.
- **Later-page RPC error:** page one returns an object and a token; page two returns UNAVAILABLE. The collector must return an error, not a successful partial collection.
- **Repeated token:** the server repeats the same continuation token. The test applies a short timeout and records whether the stream keeps requesting pages. This is a diagnostic test for the pinned SDK behavior, not a passing completeness guarantee.

### Source-level observation and risk

Inspection of the pinned upstream sui-rpc 0.4.0 client/lists.rs shows that list_owned_objects updates the request page_token from each response and continues while a token is present. The implementation does not maintain a set of previously observed tokens or otherwise visibly reject a repeated token. Therefore a server that repeats a token can cause the stream to continue issuing requests. The local repeated-token fixture is intended to reproduce and bound this condition.

This is a pagination-progress risk in the SDK implementation. It is not evidence that Sui testnet currently returns repeated tokens. The spike does not modify upstream SDK code; any production mitigation should be designed and tested separately, with a strict request/page budget and an explicit incomplete/error outcome.
