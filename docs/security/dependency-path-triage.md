# Dependency audit path triage — 2026-10-31

## Scope and provenance

- Repository: `navedjuwale-creator/7k-sdk-rs`
- Branch: `security/clean-audit-baseline`
- Baseline commit: `8a8b099acc99f9057f1f0be89a70cd48aeaac12a`
- Evidence: GitHub Actions run `41183168231`, artifact `dependency-audit-evidence` (SHA-256 `8fd62176ae005108d8a2544d334d0fa08f8f773d8264e0c589c91a0f02d87502`).
- The artifact contains `dependency-tree.txt` and `dependency-paths.txt`; the latter provides reverse dependency paths for 27 package/version entries.
- The audit log scanned 715 locked crate dependencies and reported 36 vulnerability entries, plus unmaintained/unsound/yanked advisories. Counts are findings, not unique packages; a package can have multiple advisories.

## Interpretation

The “Immediate dependent(s)” column records the first package(s) directly above the affected crate in Cargo’s reverse dependency tree. These are dependency-path evidence, not proof that every listed parent is exploitable or that the project directly declared the affected crate. Multiple paths can exist; the source artifact remains the authoritative full path record.

## Reverse dependency inventory

| Affected crate | Locked version | Immediate dependent(s) from captured paths |
|---|---:|---|
| `aes-gcm` | `0.10.1` | `fastcrypto v0.1.8` |
| `bytes` | `1.8.0` | `anemo v0.0.0`, `axum v0.7.5`, `axum-core v0.4.3`, `axum-server v0.6.1`, `h2 v0.3.26`, `h2 v0.4.5`, `http v0.2.9`, `http v1.1.0`, `http-body v0.4.5`, `http-body v1.0.1`, `http-body-util v0.1.2`, `hyper v0.14.26`, `hyper v1.4.1`, `hyper-tls v0.6.0`, `hyper-util v0.1.10`, `mysten-network v0.2.0`, `object_store v0.10.2`, `prost v0.13.3`, `protobuf v2.28.0`, `quinn v0.11.2`, `quinn-proto v0.11.8`, `reqwest v0.12.15`, `soketto v0.7.1`, `sui-rpc-api v0.1.0`, `tokio v1.54.1`, `tokio-util v0.7.10`, `tonic v0.12.3`, `tower-http v0.5.2`, `tungstenite v0.21.0` |
| `core2` | `0.4.0` | `multihash v0.17.0` |
| `crossbeam-epoch` | `0.9.18` | `crossbeam-deque v0.8.5` |
| `crypto-bigint` | `0.4.9` | `elliptic-curve v0.12.3`, `rfc6979 v0.3.1` |
| `crypto-bigint` | `0.5.1` | `elliptic-curve v0.13.8` |
| `ethnum` | `1.5.3` | `move-core-types v0.0.4` |
| `futures-util` | `0.3.30` | `axum v0.7.5`, `axum-core v0.4.3`, `axum-server v0.6.1`, `futures v0.3.29`, `futures-executor v0.3.29`, `h2 v0.3.26`, `http-body-util v0.1.2`, `hyper v0.14.26`, `hyper v1.4.1`, `hyper-rustls v0.27.2`, `hyper-util v0.1.10`, `jsonrpsee-client-transport v0.16.2`, `jsonrpsee-core v0.16.2`, `jsonrpsee-server v0.16.2`, `reqwest v0.12.15`, `tokio-tungstenite v0.21.0`, `tower v0.4.13`, `tower v0.5.3`, `tower-http v0.5.2` |
| `h2` | `0.3.26` | `hyper v0.14.26` |
| `h2` | `0.4.5` | `hyper v1.4.1`, `reqwest v0.12.15`, `tonic v0.12.3` |
| `idna` | `0.5.0` | `url v2.5.2` |
| `protobuf` | `2.28.0` | `prometheus v0.13.3`, `prometheus-closure-metric v0.1.0` |
| `quick-xml` | `0.36.1` | `object_store v0.10.2` |
| `quinn-proto` | `0.11.8` | `anemo v0.0.0`, `quinn v0.11.2` |
| `ring` | `0.16.20` | `rustls v0.20.9`, `sct v0.7.0` |
| `ring` | `0.17.8` | `anemo v0.0.0`, `object_store v0.10.2`, `quinn-proto v0.11.8`, `rcgen v0.13.1`, `rustls v0.23.12`, `rustls-webpki v0.102.6`, `webpki v0.22.4` |
| `rsa` | `0.8.2` | `fastcrypto v0.1.8` |
| `rustls` | `0.20.9` | `hyper-rustls v0.23.2`, `tokio-rustls v0.23.4` |
| `rustls` | `0.23.12` | `anemo v0.0.0`, `axum-server v0.6.1`, `hyper-rustls v0.27.2`, `quinn v0.11.2`, `quinn-proto v0.11.8`, `reqwest v0.12.15`, `sui-tls v0.0.0`, `tokio-rustls v0.26.0` |
| `rustls-webpki` | `0.102.6` | `anemo v0.0.0`, `rustls v0.23.12`, `sui-tls v0.0.0` |
| `spin` | `0.9.8` | `lazy_static v1.5.0`, `ring v0.17.8` |
| `time` | `0.3.36` | `asn1-rs v0.5.1`, `rcgen v0.13.1`, `x509-parser v0.14.0`, `yasna v0.5.2` |
| `webpki-roots` | `0.22.6` | `hyper-rustls v0.23.2`, `jsonrpsee-client-transport v0.16.2` |
| `webpki-roots` | `0.26.3` | `hyper-rustls v0.27.2`, `reqwest v0.12.15`, `tonic v0.12.3` |
| `zstd` | `0.13.2` | `async-compression v0.4.6` |
| `zstd-safe` | `7.2.1` | `async-compression v0.4.6`, `zstd v0.13.2` |
| `zstd-sys` | `2.0.11+zstd.1.5.6` | `zstd-safe v7.2.1` |

## Remediation order

1. **Transport/security stack:** `quinn-proto`, `rustls`, `rustls-webpki`, `ring`, and `webpki-roots`. These have security-relevant network/TLS findings and several upstream consumers (including `anemo`, `quinn`, `reqwest`, `tonic`, and Sui TLS/network crates). Resolve through compatible upstream dependency updates; do not patch transitive lockfile versions blindly.
2. **Parsing and compression:** `quick-xml`, `zstd`, `zstd-safe`, and `zstd-sys`. Check the compatible version constraints in their immediate dependents (`object_store`, `async-compression`, and `zstd`).
3. **Core utility/runtime crates:** `bytes`, `h2`, `time`, `crossbeam-epoch`, `crypto-bigint`, `rsa`, `aes-gcm`, and `protobuf`. Prioritize based on reachability and actual use in the application/runtime.
4. **Maintenance/unsound/yanked advisories:** determine whether each is build-only, test-only, runtime, or target-specific before replacing; do not treat “unmaintained” as equivalent to an exploitable vulnerability.

## Guardrails for the remediation batch

- Preserve the Sui SDK pin (`testnet-v1.39.3`) until a separate compatibility decision is made.
- Preserve the current passing `cargo check --locked` baseline.
- Do not manually edit `Cargo.lock` or force versions without validating all upstream semver constraints and feature unification.
- For each proposed update, record: old/new version, parent constraint, dependency path, advisory IDs, `cargo tree -i` before/after, `cargo check --locked`, tests, and audit delta.
- Keep `main` untouched; changes remain on this security branch and PR #5 remains draft until the audit and regression checks pass.

## Next executable action

Create an isolated candidate update for the highest-priority transport/TLS group, beginning with the parent constraints for `anemo`/`quinn`/`rustls` and Sui’s pinned dependency graph. Validate the candidate in CI against the existing baseline before accepting any version change.
