# Dependency Security Baseline — 2026-10-31

## Status

**Audit failed; dependency graph is not cleared for release.** This report records the isolated CI evidence. It does not authorize merging the branch or changing production dependencies.

- Repository: `navedjuwale-creator/7k-sdk-rs`
- Base: `main` at `0b2cb6ddbe745977454c02a2b13b2c8e3ad98df1`
- Audit branch: `security/clean-audit-baseline`
- Evidence commit at report preparation: `0c99178740a060f50edd3d5ae515001352f2343c`
- Draft PR: #5 (not merged)
- The workflow-only branch began from main. It subsequently gained a seeded `Cargo.lock` copied from an earlier audit branch so the existing graph could be examined. No application source or `Cargo.toml` changes were made.

## Resolution blocker

A clean `cargo generate-lockfile` failed before it could create a new lockfile. The pinned Sui SDK tag `testnet-v1.39.3` resolves through `mysten-network -> multiaddr 0.17.0 -> multihash 0.17.0`, which requires `core2 = ^0.4.0`; crates.io reports `core2 0.4.0` as yanked.

This means the current manifest cannot be freshly resolved by the tested command without a deliberate dependency-graph/lockfile decision. Do not solve this by silently allowing yanked dependencies or changing the Sui tag without compatibility checks.

## Seeded lockfile audit result

GitHub Actions run: https://github.com/navedjuwale-creator/7k-sdk-rs/actions/runs/41166646374/job/124435636651

- `cargo metadata --locked --format-version 1`: passed.
- Dependency tree capture: passed.
- `cargo-audit`: failed.
- Reported: **36 vulnerabilities** and **33 allowed warnings** across **715 crate dependencies**.
- `cargo check --locked`: skipped because the audit step failed first; compilation is therefore unverified by this run.

The seeded lockfile is inherited evidence, not a fresh resolution of the current manifest. Its audit is useful for triage but must not be described as a newly resolved dependency inventory.

## High-priority items reported by the seeded audit

The audit output included vulnerable versions of (non-exhaustive summary):

- `bytes 1.8.0`
- `h2 0.3.26` and `h2 0.4.5`
- `quick-xml 0.36.1` (reported twice)
- `quinn-proto 0.11.8` (multiple advisory findings)
- `rustls 0.20.9` and `rustls 0.23.12`
- `rustls-webpki 0.102.6`
- `time 0.3.36`
- `zstd-safe 7.2.1` (multiple unsoundness advisories)
- `core2 0.4.0`, `futures-util 0.3.30`, and `spin 0.9.8` are also flagged as yanked.

Other reported packages include `aes-gcm`, `crossbeam-epoch`, `crypto-bigint`, `idna`, `protobuf`, `ring`, `rsa`, `webpki-roots`, `zstd`, and `zstd-sys`. Consult the linked Actions log for the complete advisory details and affected versions. A package/advisory finding is not, by itself, proof that the affected code path is reachable in this application.

## Safe next actions

1. Preserve `main` and keep PR #5 in draft.
2. Create a machine-readable inventory of all audit findings and the dependency paths that introduce each affected crate.
3. Classify each finding by direct/transitive dependency, available fixed version, semver/feature constraints, and whether the path is reachable.
4. Investigate the yanked `core2` chain separately. Evaluate an upstream Sui SDK/tag update or a carefully reviewed lockfile strategy; do not add a speculative direct dependency override.
5. Apply narrowly scoped updates on a separate branch, then run audit and compilation independently. Do not skip the compile check merely because the audit fails.

## Interpretation

This is an evidence-based triage baseline, **not a passing security check**. No vulnerability suppression has been added, and no changes have been merged into `main`.
