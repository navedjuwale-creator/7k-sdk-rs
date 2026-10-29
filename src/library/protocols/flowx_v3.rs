use super::get_adjusted_sqrt_price_limit;
use crate::{
    library::swap_with_route::{Swapper, ToTypeTags, parse_amount, required_extra_value},
    utils::{sui::ObjectRefFetcher, token::normalize_token_type},
};
use anyhow::Result;
use sui_sdk::types::transaction::Argument;

pub async fn swap<'a>(swapper: &mut Swapper<'a>) -> Result<Argument> {
    let swap_x_to_y = swapper.swap.swap_x_to_y;

    let swap_fee_rate = parse_amount(
        required_extra_value(&swapper.swap.swap, "swap_fee_rate")?,
        "FlowX V3 swap_fee_rate",
    )?;

    let config = &swapper.config.flowx_v3;

    let registry_obj = swapper.client.shared_obj_mut(&config.registry).await?;
    let version_obj = swapper.client.shared_obj_mut(&config.version).await?;
    let clock = swapper.tx.clock()?;

    let pool_tokens = &swapper.swap.pool.all_tokens;
    let coin_x_type = normalize_token_type(&pool_tokens[0].address);
    let coin_y_type = normalize_token_type(&pool_tokens[1].address);

    let type_tags = if swap_x_to_y {
        vec![coin_x_type, coin_y_type]
    } else {
        vec![coin_y_type, coin_x_type]
    }
    .to_type_tags()?;

    let sqrt_price_limit = get_adjusted_sqrt_price_limit(swap_x_to_y);

    let args = vec![
        swapper.tx.obj(registry_obj)?,
        swapper.tx.pure(swap_fee_rate)?,
        *swapper.input_coin_object,
        swapper.tx.pure(0u64)?,
        swapper.tx.pure(sqrt_price_limit)?,
        swapper.tx.pure(u64::MAX)?,
        swapper.tx.obj(version_obj)?,
        clock,
    ];

    let coin_out = swapper.tx.move_call(
        &config.base.package,
        "swap_router",
        "swap_exact_input",
        type_tags,
        args,
    )?;

    Ok(coin_out)
}
