# Sui gRPC Stage 3 — Network and Object Conversion Contract

Status: **Design only. No transaction-facing implementation is authorized by this stage.**

Branch: `migrate/sui-grpc-client`

## 1. Purpose and non-goals

Stage 3 defines the contract that a future adapter must satisfy before the standalone gRPC read-only model can be connected to the legacy application.

In scope:
- explicit, deterministic network selection;
- endpoint identity verification and chain-bound object records;
- a typed conversion decision boundary from neutral `SuiObjectRecord` metadata to legacy object arguments;
- fail-closed behavior and a test acceptance matrix.

Out of scope:
- changing `src/client.rs` or `src/utils/sui.rs`;
- constructing programmable transactions or `ObjectArg` values in production code;
- simulation, gas estimation, signing, broadcast, or writes;
- changing dependency versions or resolving the legacy SDK / `sui-rpc` dependency conflict;
- PR creation, merge, or changes to `main`.

## 2. Network-selection contract

### 2.1 Required caller input

The future public entry point must require a caller-supplied configuration. There is no default network, no implicit localhost fallback, and no selection based only on a human-readable label.

Conceptual shape (design notation, not compiled API):

```rust
enum SuiNetworkKind {
    Mainnet,
    Testnet,
    Devnet,
    Localnet,
    Custom,
}

struct SuiNetworkConfig {
    kind: SuiNetworkKind,
    endpoint: Url,
    expected_chain_id: ChainId,
    // Explicit opt-in only for a local development endpoint if HTTP is needed.
    allow_insecure_local_transport: bool,
}
```

The actual implementation may use different names, but it must preserve these semantics.

### 2.2 Validation and connection rules

1. Require a non-empty, syntactically valid endpoint and an explicit expected chain ID.
2. Require HTTPS for Mainnet, Testnet, Devnet, and Custom endpoints.
3. Permit HTTP only for Localnet and only when the caller explicitly opts in. Never downgrade HTTPS automatically.
4. Construct exactly one client for the selected endpoint. Do not construct unrelated network clients as a side effect.
5. Call `GetServiceInfo` before returning a usable reader/client.
6. Require a non-empty observed chain ID and exact equality with the configured expected chain ID. Fail closed on missing, malformed, or mismatched identity.
7. Treat `chain` / network labels as diagnostics only; the chain ID is the identity key.
8. Keep endpoint and verified chain identity together in an immutable connection context. Every object record and later conversion request must carry or reference that context.
9. Never reuse cached object metadata across chain IDs. Endpoint aliases that report the same chain ID still must not bypass endpoint/config validation.
10. Do not infer Mainnet/Testnet from URL substrings, environment defaults, API versions, or user-facing labels.

### 2.3 Failure behavior

Connection must fail before any object lookup if:
- endpoint parsing or transport policy fails;
- a public network is configured with HTTP;
- Localnet HTTP is requested without explicit opt-in;
- expected chain ID is missing;
- service identity cannot be read before the deadline;
- observed chain ID is missing or differs from the configured value.

There must be no retry against another network and no fallback to localhost.

### 2.4 Acceptance tests

- each named network requires an explicit endpoint and expected chain ID;
- no zero-argument/default constructor can create a production connection;
- Mainnet/Testnet/Devnet reject HTTP;
- Localnet rejects HTTP without explicit opt-in;
- exactly the selected endpoint is used;
- service-info success with matching chain ID succeeds;
- missing, empty, malformed, or mismatched chain ID fails closed;
- timeout and transport failures fail closed;
- no object read is attempted after failed identity verification.

## 3. Chain-bound object record contract

The neutral Stage 2 `SuiObjectRecord` remains the read boundary. It must preserve, without lossy conversion:

- verified chain ID;
- canonical object ID;
- object version and digest;
- object type;
- exact owner variant and its associated metadata;
- optional BCS contents only when explicitly requested.

A conversion must reject a record whose chain ID differs from the active verified connection context. It must also reject malformed IDs/digests, absent required metadata, zero/invalid versions, or records whose requested object ID does not match the returned ID.

A plain legacy `ObjectRef` is insufficient as the sole input to conversion because it does not preserve owner kind or exact shared initial version.

## 4. Object-owner conversion decision matrix

The future conversion API must be typed and explicit. It must not infer owner kind from caller intent, object type, or a requested mutability flag.

| Neutral owner kind | Potential legacy representation | Contract decision |
|---|---|---|
| Address | `ImmOrOwnedObject(ObjectRef)` | Conditional: allow only after validating the record is address-owned by the exact expected sender/owner and the reference is fresh and chain-bound. |
| Immutable | `ImmOrOwnedObject(ObjectRef)` | Conditional read-only representation only; no mutable intent. Require an explicit immutable conversion path and verified immutable owner metadata. |
| Shared | `SharedObject { id, initial_shared_version, mutable }` | Conditional: use the exact `initial_shared_version` from the record. Mutability must be explicit caller intent, never inferred from the read. Reject missing/zero version. |
| Object-owned | No generic mapping approved | Reject for now. Do not silently treat it as address-owned or shared. A future Receiving/object-parent contract must be separately specified and tested against the selected SDK/PTB version. |
| Consensus-address-owned | No generic mapping approved | Reject for now. Preserve address and exact start version in the neutral model; do not coerce to ordinary address-owned or shared. Require a dedicated SDK-supported representation and tests before enabling. |

### 4.1 Explicit mutability

The future conversion request should express intent independently from the read record, conceptually:

```rust
enum SharedAccess {
    ReadOnly,
    Mutable,
}
```

This request is valid only for a `Shared` owner. It must never turn an address-owned, immutable, object-owned, or consensus-address-owned object into a shared argument. Read-only is not a promise that later transaction execution cannot mutate through another alias; it is only the PTB access mode requested by the caller.

### 4.2 Freshness and sender checks

For address-owned conversion:
- caller supplies the expected sender address;
- record owner must equal that address exactly;
- object reference must be bound to the verified chain ID;
- stale object references must be refreshed before a transaction is ever built;
- no ownership assumption may be inferred from the fact that an object was returned by an owner inventory query.

For immutable conversion:
- immutable ownership must be explicit in the record;
- mutable shared access is impossible and must be rejected.

For shared conversion:
- preserve the exact initial shared version;
- never use `OBJECT_START_VERSION` as a substitute;
- reject a request to convert a non-shared owner as shared.

### 4.3 Conversion result

The eventual conversion function should return a typed error for every rejected case, not `Option`, a default object argument, or a fallback. Error categories should distinguish:
- chain mismatch;
- invalid or incomplete object reference;
- sender/owner mismatch;
- unsupported owner kind;
- missing or invalid shared initial version;
- illegal mutability request;
- unsupported legacy SDK representation.

No conversion implementation belongs in Stage 3.

## 5. SDK and dependency boundary

The Stage 2 crate uses `sui-rpc = 0.4.0`; the root application uses the older pinned `sui-sdk` and legacy PTB types. The earlier root dependency attempt exposed an upstream dependency conflict.

Therefore:
- keep `SuiObjectRecord` and its owner model independent of transaction SDK types;
- do not add `sui-rpc` directly to the root manifest as an unreviewed shortcut;
- do not transmute or serialize one SDK's object/transaction types into another SDK's types;
- before implementation, prove exact versions, feature flags, type compatibility, and build impact in an isolated dependency experiment;
- if no safe narrow compatibility layer exists, prefer a separately staged transaction SDK migration over a broad import replacement.

## 6. Required test matrix before any transaction-facing integration

### Network
- explicit selection for Mainnet, Testnet, Devnet, Localnet, and Custom;
- expected chain ID mandatory;
- transport policy enforcement;
- matching/mismatching/missing chain ID;
- connection timeout and service-info errors;
- proof no fallback or second endpoint is attempted.

### Object conversion
- address owner + matching sender: accepted only by the explicit owned path;
- address owner + different sender: rejected;
- immutable owner + immutable path: accepted as read-only only;
- immutable owner + mutable request: rejected;
- shared owner + exact initial version: preserved exactly;
- shared owner + read-only and mutable intent: each explicit and separately tested;
- shared owner with missing/zero initial version: rejected;
- non-shared owner requested as shared: rejected;
- object-owned owner: rejected until Receiving semantics are separately implemented;
- consensus-address owner: rejected until dedicated support is proven;
- wrong chain ID, malformed ID/digest, zero version, missing owner metadata: rejected;
- stale reference: refresh required before any transaction construction.

### Build and regression
- formatting and all existing Stage 1/Stage 2 tests remain green;
- isolated compile test against the exact legacy PTB version;
- no root dependency change until compatibility is demonstrated;
- `main` remains unchanged.

## 7. Stage 3 exit criteria and next gate

Stage 3 design is complete when:
1. network selection and identity rules are accepted as a written contract;
2. owner conversion matrix and all rejection cases are explicit;
3. test cases above are mapped to a future isolated implementation plan;
4. dependency compatibility is treated as a separate gate;
5. no transaction-facing code has changed.

Only after those criteria should a later stage implement a minimal, pure conversion module with unit tests. That module must not build a transaction. PTB integration, simulation, signing, and broadcast each require their own later review and authorization.

## 8. Current known implementation hazards (source evidence)

- `src/client.rs::get_sui_client` currently constructs localnet, testnet, and devnet clients, then unconditionally returns localnet.
- `src/utils/sui.rs::ObjectRefFetcher::object_ref` returns only the legacy ID/version/digest reference and discards owner metadata.
- `src/utils/sui.rs::ObjectArgExt::shared_obj` substitutes `OBJECT_START_VERSION` instead of using the object's actual shared initial version.
- Stage 2's neutral `SuiOwner` model preserves Address, Object, Shared (initial shared version), Immutable, and ConsensusAddress (address and start version); it is not yet converted into legacy PTB types.

## 9. Safety and repository boundaries

This is a design document only. The legacy client and PTB source files remain unchanged. No transaction is constructed, simulated, signed, or broadcast. No on-chain write is attempted. No PR is opened and no merge is performed. Production `main` remains untouched.
