# 7k-rs-sdk

A Rust SDK for the 7k Aggregator on Sui.

## Sui network configuration

The Sui client reads the `SUI_NETWORK` environment variable when `get_sui_client()` is called.

Supported values:

- `localnet` — default when `SUI_NETWORK` is unset; a local Sui node must be running.
- `testnet` — connect to Sui testnet.
- `devnet` — connect to Sui devnet.
- `mainnet` — currently rejected until the SDK is migrated to a supported gRPC or GraphQL transport.

Set the variable before starting your application:

```sh
export SUI_NETWORK=testnet
cargo run
```

If unset, the SDK selects localnet. Unsupported values and environment-variable read errors are returned as errors; the SDK does not silently fall back to another network. Network selection does not start or configure a local Sui node.
