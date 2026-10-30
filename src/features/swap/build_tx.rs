use std::str::FromStr;

use crate::{
    consts::{_7K_CONFIG, _7K_PACKAGE_ID, _7K_VAULT},
    library::{
        group_swap_routes::group_swap_routes,
        swap_with_route::{ToTypeTags, swap_with_route},
    },
    types::tx::BuildTxParams,
    utils::{
        sui::{ArgumentExt, ObjectRefFetcher, Ptb},
        token::denormalize_token_type,
    },
};
use anyhow::Result;
use sui_sdk::{
    SuiClient,
    types::{base_types::SuiAddress, transaction::Argument},
};

use super::config::ConfigManager;

pub async fn build_tx(
    client: &SuiClient,
    config_manager: &mut ConfigManager,
    params: BuildTxParams,
) -> Result<(Ptb, Option<Argument>)> {
    let BuildTxParams {
        common: common_params,
        dev_inspect,
    } = params;

    let account_address = &common_params.account_address;
    let quote_response = common_params.quote_response;
    let slippage = common_params.slippage;
    let commission = common_params.commission;
    let extend_tx = common_params.extend_tx;

    if account_address.is_empty() {
        return Err(anyhow::anyhow!("Sender address is required"));
    }

    if quote_response.routes.is_none() {
        return Err(anyhow::anyhow!(
            "Invalid quote response: 'routes' are required"
        ));
    }

    // Validate commission partner address
    if !SuiAddress::from_str(&commission.partner).is_ok() {
        return Err(anyhow::anyhow!("Invalid commission partner address"));
    }

    let is_extended = extend_tx.is_some();
    let (mut tx, coin_in) = match extend_tx {
        Some(ext) => (ext.tx, ext.coin_in),
        None => (Ptb::new(), None),
    };

    let routes = group_swap_routes(&quote_response)?;
    let splits: Vec<u64> = routes
        .iter()
        .map(|group| {
            group
                .first()
                .as_ref()
                .unwrap()
                .swap
                .amount
                .parse::<u64>()
                .unwrap()
        })
        .collect();

    let coins_arg = if let Some(coin_in) = coin_in {
        let split_coins = tx.split_coins(coin_in, &splits)?;
        tx.transfer_or_destroy_zero_coin(
            quote_response.token_in.as_str(),
            coin_in,
            Some(SuiAddress::from_str(account_address)?),
        )?;
        split_coins
    } else {
        let split_result = tx
            .get_split_coin_for_tx(
                client,
                SuiAddress::from_str(account_address)?,
                quote_response.swap_amount.parse::<u64>().unwrap(),
                &splits,
                denormalize_token_type(&quote_response.token_in),
                dev_inspect,
            )
            .await?;
        split_result
    };

    let mut coin_objects = Vec::new();
    let config = config_manager.get_config().await?;

    for (index, route) in routes.iter().enumerate() {
        let input_coin_object = coins_arg.get_slice(index as u16)?;

        let coin_res = swap_with_route(
            client,
            route,
            input_coin_object,
            account_address,
            &config,
            &mut tx,
        )
        .await?;

        coin_objects.push(coin_res);
    }

    let mut coin_out = None;
    if !coin_objects.is_empty() {
        let merge_coin = if coin_objects.len() > 1 {
            tx.merge_coins(coin_objects)
        } else {
            coin_objects[0].clone()
        };

        coin_out = Some(merge_coin.clone());

        let min_received =
            ((1.0 - slippage) * quote_response.return_amount.parse::<f64>()?).round() as u64;

        let partner_addy = tx.pure(commission.partner.clone())?;

        let partner = tx.move_call(
            "0x1",
            "option",
            "some",
            vec!["address"].to_type_tags()?,
            vec![partner_addy],
        )?;

        let args = vec![
            tx.obj(client.shared_obj_mut(_7K_CONFIG).await?)?,
            tx.obj(client.shared_obj_mut(_7K_VAULT).await?)?,
            tx.pure(quote_response.swap_amount_with_decimal)?,
            merge_coin,
            tx.pure(min_received)?,
            tx.pure(quote_response.return_amount_with_decimal)?,
            partner,
            tx.pure(commission.commission_bps)?,
        ];

        tx.move_call(
            _7K_PACKAGE_ID,
            "settle",
            "settle",
            vec![
                quote_response.token_in.as_str(),
                quote_response.token_out.as_str(),
            ]
            .to_type_tags()?,
            args,
        )?;

        // Handle commission and settlement logic here
        // This is a simplified version - actual implementation would need proper Move call handling
        if !is_extended {
            let addy = tx.pure(account_address)?;
            // Transfer objects if not an extended transaction
            tx.transfer_objects(vec![merge_coin], addy)?;
        }
    }

    Ok((tx, coin_out))
}
