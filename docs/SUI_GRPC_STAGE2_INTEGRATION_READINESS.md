# Sui gRPC Stage 2 — Integration Readiness Review

Status: **Stage 2 read-only adapter verified; not ready to wire into the legacy transaction layer.**

Reviewed after commit `20d19dfe55fc3758471247a437496453ce58d138` and live CI run [41101192339](https://github.com/navedjuwale-creator/7k-sdk-rs/actions/runs/41101192339). A follow-up cleanup removes one unused test helper; it does not change runtime behavior.

## Verified evidence

- The standalone `sui-grpc-inventory` crate compiles and its deterministic tests pass.
- The Stage 2 workflow verifies the public `GetObject` request field masks, chain-ID validation, object metadata validation, and all five supported owner forms.
- Live read-only Testnet smoke test connected to `https://fullnode.testnet.sui.io:443`, verified the pinned chain ID `69WiPg3DAQiwdxfncX6wYQ2siKwAe6L9BZthQea3JNMD`, and read Sui system object `0x...0005`.
- Live result: object version `1070563932`, type `0x...0003::sui_system::SuiSystemState`.
- No transaction construction, signing, broadcast, or on-chain write was performed.

The live smoke test proves service identity and one public object read. It does not prove every object/owner case against live network data; those cases remain covered by deterministic response tests.

## Integration blockers in the legacy application

### 1. Network selection is not explicit

`src/client.rs::get_sui_client` constructs localnet, testnet, and devnet clients, then returns the localnet client unconditionally. The function's TODO also conflicts with its current behavior. A gRPC adapter must not be wired behind this ambiguous network selector.

**Required:** replace implicit selection with an explicit network/endpoint configuration and a verified chain identity. Do not silently fall back to localhost or another network.

### 2. Legacy object reads discard owner metadata

`src/utils/sui.rs::ObjectRefFetcher::object_ref` returns only the legacy `ObjectRef` (ID/version/digest). It cannot carry the owner variant or exact shared-object initial version now preserved by `SuiObjectRecord`.

### 3. Shared-object conversion is unsafe as a general adapter

`ObjectArgExt::shared_obj` assigns `OBJECT_START_VERSION` to every shared object. The Stage 2 gRPC reader preserves the actual `initial_shared_version`, but there is currently no safe conversion contract from `SuiOwner` to legacy `ObjectArg`. Consensus-address ownership also has semantics that must not be coerced into ordinary address or shared ownership.

**Required:** implement a separately reviewed conversion boundary that accepts only supported owner kinds, preserves exact initial shared version, and takes mutability as explicit caller intent. Reject unsupported owner kinds and invalid conversions.

### 4. Dependency and transaction-type boundary remains unresolved

The standalone crate uses `sui-rpc = 0.4.0`; the root application still uses the older pinned `sui-sdk` and legacy PTB types. A previous direct root dependency attempt exposed an upstream dependency conflict. Stage 2 does not establish transaction-type or serialization compatibility.

**Required:** design and test a narrow compatibility boundary or migrate the transaction representation in a later isolated stage. Do not solve this by replacing imports wholesale or by adding the gRPC dependency directly to the root without a compatibility proof.

## Decision

**Do not integrate Stage 2 directly into `src/utils/sui.rs` or the PTB layer yet.** Preserve the verified standalone adapter and keep `main` unchanged.

Recommended next step: Stage 3 design review for explicit network selection and a typed, read-only object-reference conversion contract. That review should first specify supported owner cases, exact shared-version handling, mutability intent, chain binding, and dependency boundaries. Only then implement a minimal adapter with tests. Simulation/gas migration and transaction construction remain separate later gates.

## Safety boundary

This branch remains isolated. No PR or merge is created. No transaction is built, signed, or broadcast, and no on-chain write is attempted.
