use crate::{
    destruct,
    library::swap_with_route::{parse_amount, required_extra_value, Swapper, ToTypeTags},
    utils::sui::{ArgumentExt, ObjectRefFetcher},
};
use anyhow::Result;
use std::str::FromStr;
use sui_sdk::types::{base_types::SuiAddress, transaction::Argument};

pub async fn swap<'a>(swapper: &mut Swapper<'a>) -> Result<Argument> {
    let swap_x_to_y = swapper.swap.swap_x_to_y;
    let client_order_id = chrono::Utc::now().timestamp_millis() as u64;

    let type_args = swapper.get_type_params()?;

    let base_asset = type_args.get(0).unwrap();
    let quote_asset = type_args.get(1).unwrap();

    let account_cap = create_account_cap(swapper).await?;
    let amount_in = swapper.get_input_coin_value()?;

    let c = &swapper.client;
    let cfg = &swapper.config.deepbook;
    let pool_id = c.shared_obj_mut(&swapper.swap.swap.pool_id).await?;

    let lot_size = parse_amount(
        required_extra_value(&swapper.swap.swap, "lot_size")?,
        "DeepBook lot_size",
    )?;
    if lot_size == 0 {
        anyhow::bail!("DeepBook lot_size must be greater than zero");
    }

    let result;

    if swap_x_to_y {
        let args = vec![amount_in, swapper.tx.pure(lot_size)?];

        let amount_in_round =
            swapper
                .tx
                .move_call(&cfg.package, "math", "m_round_down", vec![], args)?;

        let args = vec![
            swapper.tx.obj(pool_id)?,
            swapper.tx.pure(client_order_id)?,
            account_cap.clone(),
            amount_in_round,
            *swapper.input_coin_object,
            swapper.tx.move_call(
                "0x2",
                "coin",
                "zero",
                vec![quote_asset.as_str()].to_type_tags()?,
                vec![],
            )?,
            swapper.tx.clock()?,
        ];

        // let (base_coin_ret, quote_coin_ret) = swapper.tx.move_call(
        let res = swapper
            .tx
            .move_call(
                &cfg.package,
                "clob_v2",
                "swap_exact_base_for_quote",
                vec![base_asset.as_str(), quote_asset.as_str()].to_type_tags()?,
                args,
            )?
            .split(2)?;

        let (base_coin_ret, quote_coin_ret) = destruct!(2, res);

        delete_account_cap(swapper, account_cap).await?;
        swapper.tx.transfer_or_destroy_zero_coin(
            &swapper.swap.swap.asset_in,
            base_coin_ret,
            Some(SuiAddress::from_str(swapper.current_account)?),
        )?;
        result = quote_coin_ret;
    } else {
        let args = vec![
            swapper.tx.obj(pool_id.clone())?,
            swapper.tx.pure(client_order_id)?,
            account_cap.clone(),
            amount_in,
            swapper.tx.clock()?,
            *swapper.input_coin_object,
        ];

        let res = swapper
            .tx
            .move_call(
                &cfg.package,
                "clob_v2",
                "swap_exact_quote_for_base",
                vec![base_asset.as_str(), quote_asset.as_str()].to_type_tags()?,
                args,
            )?
            .split(2)?;

        let (base_coin_ret, quote_coin_ret) = destruct!(2, res);

        delete_account_cap(swapper, account_cap).await?;
        swapper.tx.transfer_or_destroy_zero_coin(
            &swapper.swap.swap.asset_in,
            quote_coin_ret,
            Some(SuiAddress::from_str(swapper.current_account)?),
        )?;
        result = base_coin_ret;
    }

    Ok(result)
}

async fn create_account_cap<'a>(swapper: &mut Swapper<'a>) -> Result<Argument> {
    let cap = swapper.tx.move_call(
        &swapper.config.deepbook.package,
        "clob_v2",
        "create_account",
        vec![],
        vec![],
    )?;
    Ok(cap)
}

async fn delete_account_cap<'a>(swapper: &mut Swapper<'a>, account_cap: Argument) -> Result<()> {
    swapper.tx.move_call(
        &swapper.config.deepbook.package,
        "custodian_v2",
        "delete_account_cap",
        vec![],
        vec![account_cap],
    )?;
    Ok(())
}
