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


## Bounded direct-pagination adapter

A separate test adapter now calls the generated `StateServiceClient::list_owned_objects` unary RPC directly rather than using the SDK's opaque auto-paginating stream. This keeps the upstream `sui-rpc = 0.4.0` dependency unchanged while making page tokens visible to the collector.

The adapter:
- tracks previously submitted continuation tokens and rejects a repeated token;
- enforces a caller-supplied maximum page count;
- follows continuation tokens even when a page contains no objects;
- returns an RPC error or explicit limit/cycle error rather than exposing accumulated objects as a successful complete result.

Local mock-gRPC regression tests cover normal two-page completion, repeated-token rejection, page-budget exhaustion, and a later-page RPC failure. The existing live testnet probe still uses the SDK's high-level stream and remains a bounded 1,000-object sample. A separate ignored integration test exercises this direct adapter against testnet using the SDK client's configured `state_client()` transport.

**Integration boundary:** the adapter is a tested primitive, not yet a production collector. The live probe obtains the generated state client from the SDK's configured `Client::state_client()` transport. It sets `page_size=1000` (the protocol maximum; the default is 50), enforces an 8-page budget and a 30-second overall deadline, preserves request filters across pages, and treats every error or exhausted budget as incomplete. No mainnet or user-wallet access is performed here.


## Live bounded-adapter testnet observation

The first live run compiled successfully and all 15 ordinary tests passed. The bounded adapter integration then failed closed with `PageLimitExceeded { max_pages: 8 }`. This is evidence that the request still had a continuation token after eight pages; it is not evidence of an RPC error or natural completion. The initial request omitted `page_size`, so the protocol default of 50 entries per page applied. The probe now explicitly requests the documented maximum of 1,000 entries per page while retaining the 8-page ceiling and 30-second timeout. The next run must establish whether the larger page size reaches a natural end. If the budget is still exhausted, the result remains incomplete and must be investigated rather than relabeled as complete.

Protocol page-size contract: https://github.com/MystenLabs/sui-apis/blob/main/proto/sui/rpc/v2/state_service.proto

## Total deadline in the bounded adapter

The bounded direct-pagination adapter now accepts a single maximum duration and computes one absolute deadline before the first page request. Every page RPC is awaited with that same deadline, so slow responses across multiple pages cannot each reset the timeout. If the deadline expires, the adapter returns `DeadlineExceeded` and does not return the accumulated prefix as a complete result.

A local mock-gRPC regression test deliberately delays its first response beyond a 50 ms adapter deadline. The test asserts the explicit deadline error and verifies the call returns promptly. The live testnet probe uses a 30-second total adapter deadline in addition to its 8-page ceiling.

## Completeness hardening: explicit object bounds

The direct adapter now takes three independent bounds: maximum pages, maximum requested page size, and maximum total objects. The page-size limit must be between 1 and 1,000; the adapter overwrites the request page size with that limit so caller-provided request values cannot silently exceed the configured bound. It also checks the actual number of objects returned in each response and rejects an oversized response. Before appending a page, it checks the cumulative object count with saturating arithmetic. Any exceeded bound returns a distinct error and never returns the accumulated prefix as a successful complete collection.

The live testnet probe uses an 8-page ceiling, a 1,000-object page-size ceiling, an 8,000-object total ceiling, and the existing 30-second absolute deadline. These are independent fail-closed bounds; reaching any limit before a response with no continuation token means the inventory is incomplete.

Additional local gRPC regression coverage:
- empty intermediate page followed by a populated page in the direct adapter;
- zero page budget rejected before any RPC request;
- total-object limit exceeded on a later page, returning an error rather than a partial collection;
- invalid page-size limit rejected before any RPC request;
- server response containing more objects than the requested per-page limit, rejected before collection growth.

The upstream implementation observation is pinned to the published sui-rpc 0.4.0 source rather than a moving branch: https://docs.rs/crate/sui-rpc/0.4.0/source/src/client/lists.rs . The dependency remains exactly pinned as sui-rpc = "=0.4.0" in the isolated proof. This source reference documents the version under test; it is not a claim that the upstream SDK has been patched.
