use anyhow::{bail, Result};
// use crate::aggregator::{Coin, QuoteResponse, SorPool, SorRoute, SorSwap, TxSorSwap};
// use crate::token::denormalize_token_type;

use crate::{
    types::aggregators::{Coin, QuoteResponse, SorPool, SorRoute, SorSwap, TxSorSwap},
    utils::token::denormalize_token_type,
};

pub fn group_swap_routes(quote_response: &QuoteResponse) -> Result<Vec<Vec<TxSorSwap>>> {
    if quote_response.routes.is_none() || quote_response.swaps.len() == 0 {
        return Ok(vec![]);
    }
    let pool_details = map_pool_ids_to_details(quote_response.routes.as_ref().unwrap());
    let items: Vec<TxSorSwap> = get_tx_sor_swaps(&quote_response.swaps, &pool_details)?;
    let mut grouped_items: Vec<Vec<TxSorSwap>> = vec![];
    let mut current_group: Vec<TxSorSwap> = vec![];

    for i in 0..items.len() {
        let item = &items[i];
        current_group.push(item.clone());

        let next_item = items.get(i + 1);
        if next_item.is_none() || next_item.as_ref().unwrap().swap.amount.parse::<u64>()? > 0 {
            grouped_items.push(current_group);
            current_group = vec![];
        }
    }

    if !current_group.is_empty() {
        grouped_items.push(current_group);
    }

    Ok(grouped_items)
}

fn map_pool_ids_to_details(routes: &[SorRoute]) -> std::collections::HashMap<String, SorPool> {
    let mut pool_types: std::collections::HashMap<String, SorPool> =
        std::collections::HashMap::new();
    for route in routes {
        for hop in &route.hops {
            pool_types.insert(hop.pool_id.clone(), hop.pool.clone());
        }
    }
    pool_types
}

fn get_tx_sor_swaps(
    swaps: &[SorSwap],
    pool_details: &std::collections::HashMap<String, SorPool>,
) -> Result<Vec<TxSorSwap>> {
    swaps
        .iter()
        .map(|swap| {
            let pool = pool_details.get(&swap.pool_id).ok_or_else(|| {
                anyhow::anyhow!("Missing pool details for pool ID {}", swap.pool_id)
            })?;

            if pool.all_tokens.len() < 2 {
                bail!("Pool {} must contain at least two tokens", swap.pool_id);
            }

            let asset_in = denormalize_token_type(&swap.asset_in).to_string();
            let asset_out = denormalize_token_type(&swap.asset_out).to_string();

            let coin_x = Coin {
                coin_type: denormalize_token_type(&pool.all_tokens[0].address).to_string(),
                decimals: pool.all_tokens[0].decimal,
            };
            let coin_y = Coin {
                coin_type: denormalize_token_type(&pool.all_tokens[1].address).to_string(),
                decimals: pool.all_tokens[1].decimal,
            };

            let swap_x_to_y = if asset_in == coin_x.coin_type && asset_out == coin_y.coin_type {
                true
            } else if asset_in == coin_y.coin_type && asset_out == coin_x.coin_type {
                false
            } else {
                bail!(
                    "Swap assets do not match the token pair for pool {}",
                    swap.pool_id
                );
            };

            let mut swap_data = swap.clone();
            swap_data.asset_in = asset_in;
            swap_data.asset_out = asset_out;

            Ok(TxSorSwap {
                pool: pool.clone(),
                coin_x,
                coin_y,
                swap_x_to_y,
                swap: swap_data,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::aggregators::{SourceDex, TokenInfo};

    fn test_pool(tokens: Vec<TokenInfo>) -> SorPool {
        SorPool {
            all_tokens: tokens,
            pool_type: SourceDex::Cetus,
        }
    }

    fn token(address: &str, decimal: u8) -> TokenInfo {
        TokenInfo {
            address: address.to_string(),
            decimal,
        }
    }

    fn swap(pool_id: &str, asset_in: &str, asset_out: &str) -> SorSwap {
        SorSwap {
            pool_id: pool_id.to_string(),
            asset_in_index: 0,
            asset_out_index: 1,
            amount: "10".to_string(),
            return_amount: "9".to_string(),
            asset_in: asset_in.to_string(),
            asset_out: asset_out.to_string(),
            function_name: String::new(),
            arguments: vec![],
            extra: None,
        }
    }

    #[test]
    fn uses_each_pool_tokens_own_decimals_and_detects_x_to_y() {
        let pool = test_pool(vec![token("0xaaa::coin::A", 6), token("0xbbb::coin::B", 9)]);
        let mut pools = std::collections::HashMap::new();
        pools.insert("pool-1".to_string(), pool);

        let result = get_tx_sor_swaps(
            &[swap("pool-1", "0xaaa::coin::A", "0xbbb::coin::B")],
            &pools,
        )
        .unwrap();

        assert_eq!(result[0].coin_x.decimals, 6);
        assert_eq!(result[0].coin_y.decimals, 9);
        assert!(result[0].swap_x_to_y);
    }

    #[test]
    fn detects_y_to_x() {
        let pool = test_pool(vec![token("0xaaa::coin::A", 6), token("0xbbb::coin::B", 9)]);
        let mut pools = std::collections::HashMap::new();
        pools.insert("pool-1".to_string(), pool);

        let result = get_tx_sor_swaps(
            &[swap("pool-1", "0xbbb::coin::B", "0xaaa::coin::A")],
            &pools,
        )
        .unwrap();

        assert!(!result[0].swap_x_to_y);
    }

    #[test]
    fn rejects_missing_pool_details() {
        let pools = std::collections::HashMap::new();
        assert!(get_tx_sor_swaps(
            &[swap("missing", "0xaaa::coin::A", "0xbbb::coin::B")],
            &pools,
        )
        .is_err());
    }

    #[test]
    fn rejects_pool_with_too_few_tokens() {
        let mut pools = std::collections::HashMap::new();
        pools.insert(
            "pool-1".to_string(),
            test_pool(vec![token("0xaaa::coin::A", 6)]),
        );

        assert!(get_tx_sor_swaps(
            &[swap("pool-1", "0xaaa::coin::A", "0xbbb::coin::B")],
            &pools,
        )
        .is_err());
    }

    #[test]
    fn rejects_assets_outside_the_pool_pair() {
        let mut pools = std::collections::HashMap::new();
        pools.insert(
            "pool-1".to_string(),
            test_pool(vec![token("0xaaa::coin::A", 6), token("0xbbb::coin::B", 9)]),
        );

        assert!(get_tx_sor_swaps(
            &[swap("pool-1", "0xaaa::coin::A", "0xccc::coin::C")],
            &pools,
        )
        .is_err());
    }
}
