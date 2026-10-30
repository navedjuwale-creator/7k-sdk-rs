use std::str::FromStr;

use anyhow::{Result, anyhow};
use sui_sdk::SuiClient;
use sui_sdk::rpc_types::SuiTransactionBlockEffects;
use sui_sdk::types::base_types::SuiAddress;
use sui_sdk::types::gas::GasCostSummary;

use crate::features::prices::get_sui_price;
use crate::features::swap::build_tx::build_tx;
use crate::types::tx::{BuildTxParams, CommonParams, EstimateGasFeeParams};
use crate::utils::token::format_balance;

use super::config::ConfigManager;

pub async fn estimate_gas_fee(
    client: &SuiClient,
    config_manager: &mut ConfigManager,
    params: EstimateGasFeeParams,
) -> Result<f64> {
    let EstimateGasFeeParams { common, sui_price } = params;
    let CommonParams {
        quote_response,
        account_address,
        slippage,
        extend_tx,
        commission,
    } = common;

    if account_address.is_empty() {
        return Ok(0.0);
    }

    let (tx, _) = build_tx(
        client,
        config_manager,
        BuildTxParams {
            common: CommonParams {
                quote_response,
                account_address: account_address.clone(),
                slippage,
                extend_tx,
                commission,
            },
            dev_inspect: Some(true),
        },
    )
    .await?;

    let sui_price = match sui_price {
        Some(price) => price,
        None => get_sui_price().await?,
    };

    let sui_decimals = 9u32;

    let tx_payload = tx.complete();

    let dev_inspect = client
        .read_api()
        .dev_inspect_transaction_block(
            SuiAddress::from_str(&account_address)?,
            tx_payload,
            None,
            None,
            None,
        )
        .await?;

    let SuiTransactionBlockEffects::V1(effects) = &dev_inspect.effects;
    // let effects: &SuiTransactionBlockEffects = &dev_inspect.effects;
    if effects.status.is_err() {
        return Err(anyhow!("Sui dev-inspect failed: {:?}", effects.status));
    }

    // let MyEnum::SingleVariant(inner) = value;

    let gas_used: &GasCostSummary = &effects.gas_used;
    let fee = calculate_net_gas_fee(
        gas_used.computation_cost,
        gas_used.storage_cost,
        gas_used.storage_rebate,
    )?;

    let fee_balance = format_balance(fee, sui_decimals);
    let fee_usd = (fee_balance as f64) * sui_price;

    Ok(fee_usd)
}

fn calculate_net_gas_fee(computation_cost: u64, storage_cost: u64, storage_rebate: u64) -> Result<u64> {
    computation_cost
        .checked_add(storage_cost)
        .and_then(|gross| gross.checked_sub(storage_rebate))
        .ok_or_else(|| anyhow!("Invalid or overflowing Sui gas cost summary"))
}

#[cfg(test)]
mod tests {
    use super::calculate_net_gas_fee;

    #[test]
    fn calculates_net_gas_fee_with_rebate() {
        assert_eq!(calculate_net_gas_fee(100, 50, 20).unwrap(), 130);
    }

    #[test]
    fn rejects_gas_cost_addition_overflow() {
        assert!(calculate_net_gas_fee(u64::MAX, 1, 0).is_err());
    }

    #[test]
    fn rejects_rebate_larger_than_gross_cost() {
        assert!(calculate_net_gas_fee(10, 5, 16).is_err());
    }
}
