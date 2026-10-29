use std::str::FromStr;

use anyhow::Result;
use move_core_types::language_storage::StructTag;
use sui_sdk::{
    SuiClient,
    types::{TypeTag, transaction::Argument},
};

use crate::{
    types::aggregators::{Config, SorSwap, SourceDex, TxSorSwap},
    utils::sui::Ptb,
};

use super::protocols::{
    aftermath, bluefin, bluemove, cetus, deepbook, deepbook_v3, flowx, flowx_v3, kriya, kriya_v3,
    obric, springsui, stsui, suiswap, turbos,
};

fn parse_pool_struct_tag(extra: &std::collections::HashMap<String, String>) -> Result<StructTag> {
    let pool_struct_tag = extra
        .get("poolStructTag")
        .or_else(|| extra.get("pool_struct_tag"))
        .ok_or_else(|| anyhow::anyhow!("Missing pool struct tag in swap metadata"))?;

    StructTag::from_str(pool_struct_tag).map_err(Into::into)
}

fn parse_amount(value: &str, field: &str) -> Result<u64> {
    value.parse::<u64>().map_err(|e| {
        anyhow::anyhow!(
            "Invalid {field} '{}': expected a non-negative integer ({e})",
            value
        )
    })
}

fn validate_swap_inputs(swap: &SorSwap) -> Result<()> {
    let input_type = TypeTag::from_str(&swap.asset_in)
        .map_err(|e| anyhow::anyhow!("Invalid input asset type: {e}"))?;
    let output_type = TypeTag::from_str(&swap.asset_out)
        .map_err(|e| anyhow::anyhow!("Invalid output asset type: {e}"))?;

    if input_type == output_type {
        anyhow::bail!("Input and output assets must be different");
    }

    if swap.asset_in_index == swap.asset_out_index {
        anyhow::bail!("Input and output asset indexes must be different");
    }

    parse_amount(&swap.amount, "swap amount")?;
    parse_amount(&swap.return_amount, "return amount")?;

    sui_sdk::types::base_types::ObjectID::from_str(&swap.pool_id)
        .map_err(|e| anyhow::anyhow!("Invalid pool ID: {e}"))?;

    Ok(())
}

fn required_extra_value<'a>(swap: &'a SorSwap, key: &str) -> Result<&'a str> {
    swap.extra
        .as_ref()
        .and_then(|extra| extra.get(key))
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing or empty '{key}' in swap metadata"))
}

fn validate_protocol_inputs(item: &TxSorSwap) -> Result<()> {
    let pool_type = item.pool.pool_type;

    let minimum_type_params = match pool_type {
        SourceDex::Aftermath => Some(1),
        SourceDex::Deepbook => Some(2),
        SourceDex::DeepbookV3 => Some(1),
        SourceDex::KriyaV3 | SourceDex::Turbos => Some(0),
        _ => None,
    };

    if let Some(minimum) = minimum_type_params {
        let extra =
            item.swap.extra.as_ref().ok_or_else(|| {
                anyhow::anyhow!("Missing swap metadata for {}", pool_type.as_str())
            })?;
        let tag = parse_pool_struct_tag(extra)?;
        if tag.type_params.len() < minimum {
            anyhow::bail!(
                "{} requires at least {minimum} pool type parameter(s)",
                pool_type.as_str()
            );
        }
    }

    match pool_type {
        SourceDex::Springsui | SourceDex::Stsui => {
            if item.pool.all_tokens.len() < 2 {
                anyhow::bail!("{} requires at least two pool tokens", pool_type.as_str());
            }
        }
        SourceDex::Deepbook => {
            required_extra_value(&item.swap, "lot_size")?;
        }
        SourceDex::FlowxV3 => {
            if item.pool.all_tokens.len() < 2 {
                anyhow::bail!("FlowX V3 requires at least two pool tokens");
            }
            required_extra_value(&item.swap, "swap_fee_rate")?;
        }
        SourceDex::Obric => {
            if item.pool.all_tokens.len() < 2 {
                anyhow::bail!("Obric requires at least two pool tokens");
            }
            for key in ["x_price_id", "y_price_id"] {
                let value = required_extra_value(&item.swap, key)?;
                sui_sdk::types::base_types::ObjectID::from_str(value)
                    .map_err(|e| anyhow::anyhow!("Invalid {key} object ID: {e}"))?;
            }
        }
        _ => {}
    }

    Ok(())
}

fn validate_route_inputs(route: &[TxSorSwap]) -> Result<()> {
    if route.is_empty() {
        anyhow::bail!("Swap route must contain at least one hop");
    }

    for (index, item) in route.iter().enumerate() {
        validate_swap_inputs(&item.swap)
            .map_err(|e| anyhow::anyhow!("Invalid swap at route hop {}: {e}", index + 1))?;

        validate_protocol_inputs(item)
            .map_err(|e| anyhow::anyhow!("Invalid swap at route hop {}: {e}", index + 1))?;

        let input_type = TypeTag::from_str(&item.swap.asset_in)
            .map_err(|e| anyhow::anyhow!("Invalid input asset type: {e}"))?;
        let output_type = TypeTag::from_str(&item.swap.asset_out)
            .map_err(|e| anyhow::anyhow!("Invalid output asset type: {e}"))?;

        let pool_types = item
            .pool
            .all_tokens
            .iter()
            .map(|token| {
                TypeTag::from_str(&token.address).map_err(|e| {
                    anyhow::anyhow!("Invalid pool token type '{}': {e}", token.address)
                })
            })
            .collect::<Result<Vec<_>>>()?;

        if !pool_types.contains(&input_type) {
            anyhow::bail!(
                "Invalid swap at route hop {}: input asset is absent from pool tokens",
                index + 1
            );
        }

        if !pool_types.contains(&output_type) {
            anyhow::bail!(
                "Invalid swap at route hop {}: output asset is absent from pool tokens",
                index + 1
            );
        }
    }

    Ok(())
}

pub struct Swapper<'a> {
    pub client: &'a SuiClient,
    pub swap: &'a TxSorSwap,
    pub input_coin_object: &'a Argument,
    pub current_account: &'a str,
    pub config: &'a Config,
    pub tx: &'a mut Ptb,
}

pub trait ToTypeTags {
    fn to_type_tags(&self) -> Result<Vec<TypeTag>>;
}

impl ToTypeTags for Vec<&str> {
    fn to_type_tags(&self) -> Result<Vec<TypeTag>> {
        self.iter().map(|s| TypeTag::from_str(s)).collect()
    }
}

impl<'a> Swapper<'a> {
    pub fn get_type_params(&mut self) -> Result<Vec<String>> {
        if let Some(extra) = self.swap.swap.extra.as_ref() {
            let tag = parse_pool_struct_tag(extra)?;

            let cannonical_types: Vec<String> = tag
                .type_params
                .iter()
                .map(|ttag| ttag.to_canonical_string(true)) // todo: assuming true: has prefix?
                .collect();

            Ok(cannonical_types)
        } else {
            return Err(anyhow::anyhow!("No pool struct tag.."));
        }
    }

    pub fn get_input_coin_value(&mut self) -> Result<Argument> {
        self.tx.coin_value(
            self.swap.swap.asset_in.clone(),
            self.input_coin_object.to_owned(),
        )
    }

    pub async fn swap(&mut self) -> Result<Argument> {
        let pool_type = self.swap.pool.pool_type;

        let token_out = match pool_type {
            SourceDex::Aftermath => aftermath::swap(self).await?,
            SourceDex::Bluefin => bluefin::swap(self).await?,
            SourceDex::Bluemove => bluemove::swap(self).await?,
            SourceDex::Cetus => cetus::swap(self).await?,
            SourceDex::Deepbook => deepbook::swap(self).await?,
            SourceDex::DeepbookV3 => deepbook_v3::swap(self).await?,
            SourceDex::Flowx => flowx::swap(self).await?,
            SourceDex::FlowxV3 => flowx_v3::swap(self).await?,
            SourceDex::Kriya => kriya::swap(self).await?,
            SourceDex::KriyaV3 => kriya_v3::swap(self).await?,
            SourceDex::Obric => obric::swap(self).await?,
            SourceDex::Springsui => springsui::swap(self).await?,
            SourceDex::Stsui => stsui::swap(self).await?,
            SourceDex::Suiswap => suiswap::swap(self).await?,
            SourceDex::Turbos => turbos::swap(self).await?,
        };

        Ok(token_out)
    }
}

pub async fn swap_with_route(
    client: &SuiClient,
    route: &Vec<TxSorSwap>,
    input_coin_object: Argument,
    current_account: &String,
    config: &Config,
    tx: &mut Ptb,
) -> Result<Argument> {
    validate_route_inputs(route)?;

    let mut next_coin = input_coin_object;

    for swap in route.iter() {
        let mut swapper = Swapper {
            client,
            swap,
            input_coin_object: &next_coin,
            current_account: current_account.as_str(),
            config,
            tx,
        };

        let token_out = swapper.swap().await?;

        next_coin = token_out.clone();
    }
    Ok(next_coin)
}

#[cfg(test)]
mod swap_input_validation_tests {
    use super::{
        parse_pool_struct_tag, validate_protocol_inputs, validate_route_inputs,
        validate_swap_inputs,
    };
    use crate::types::aggregators::{Coin, SorPool, SorSwap, SourceDex, TokenInfo, TxSorSwap};
    use std::collections::HashMap;

    const SUI: &str = "0x2::sui::SUI";
    const USDC: &str =
        "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC";

    fn sample_swap(input: &str, output: &str, pool_id: &str) -> SorSwap {
        SorSwap {
            pool_id: pool_id.to_owned(),
            asset_in_index: 0,
            asset_out_index: 1,
            amount: "1".to_owned(),
            return_amount: "1".to_owned(),
            asset_in: input.to_owned(),
            asset_out: output.to_owned(),
            function_name: String::new(),
            arguments: Vec::new(),
            extra: Some(HashMap::from([(
                "pool_struct_tag".to_owned(),
                "0x2::pool::Pool<0x2::sui::SUI,0x3::coin::COIN>".to_owned(),
            )])),
        }
    }

    fn sample_route_swap(tokens: Vec<&str>) -> TxSorSwap {
        TxSorSwap {
            swap: sample_swap(SUI, USDC, "0x2"),
            pool: SorPool {
                all_tokens: tokens
                    .into_iter()
                    .map(|address| TokenInfo {
                        address: address.to_owned(),
                        decimal: 9,
                    })
                    .collect(),
                pool_type: SourceDex::Cetus,
            },
            coin_x: Coin {
                coin_type: SUI.to_owned(),
                decimals: 9,
            },
            coin_y: Coin {
                coin_type: USDC.to_owned(),
                decimals: 9,
            },
            swap_x_to_y: true,
        }
    }

    #[test]
    fn generic_cetus_route_does_not_require_protocol_metadata() {
        let mut item = sample_route_swap(vec![SUI, USDC]);
        item.swap.extra = None;
        assert!(validate_route_inputs(&[item]).is_ok());
    }

    #[test]
    fn springsui_and_stsui_require_two_pool_tokens() {
        for pool_type in [SourceDex::Springsui, SourceDex::Stsui] {
            let mut item = sample_route_swap(vec![SUI, USDC]);
            item.pool.pool_type = pool_type;
            assert!(validate_protocol_inputs(&item).is_ok());
            item.pool.all_tokens.truncate(1);
            assert!(validate_protocol_inputs(&item).is_err());
        }
    }

    #[test]
    fn aftermath_requires_a_valid_pool_struct_tag() {
        let mut item = sample_route_swap(vec![SUI, USDC]);
        item.pool.pool_type = SourceDex::Aftermath;
        item.swap.extra = None;
        assert!(validate_protocol_inputs(&item).is_err());

        item.swap.extra = Some(HashMap::from([(
            "pool_struct_tag".to_owned(),
            "not-a-struct-tag".to_owned(),
        )]));
        assert!(validate_protocol_inputs(&item).is_err());
    }

    #[test]
    fn deepbook_requires_lot_size_and_two_type_parameters() {
        let mut item = sample_route_swap(vec![SUI, USDC]);
        item.pool.pool_type = SourceDex::Deepbook;
        assert!(validate_protocol_inputs(&item).is_err());

        item.swap.extra = Some(HashMap::from([
            (
                "pool_struct_tag".to_owned(),
                "0x2::pool::Pool<0x2::sui::SUI>".to_owned(),
            ),
            ("lot_size".to_owned(), "1".to_owned()),
        ]));
        assert!(validate_protocol_inputs(&item).is_err());

        item.swap.extra = Some(HashMap::from([
            (
                "pool_struct_tag".to_owned(),
                "0x2::pool::Pool<0x2::sui::SUI,0x3::coin::COIN>".to_owned(),
            ),
            ("lot_size".to_owned(), "1".to_owned()),
        ]));
        assert!(validate_protocol_inputs(&item).is_ok());
    }

    #[test]
    fn flowx_v3_requires_fee_metadata_and_two_pool_tokens() {
        let mut item = sample_route_swap(vec![SUI, USDC]);
        item.pool.pool_type = SourceDex::FlowxV3;
        assert!(validate_protocol_inputs(&item).is_err());

        item.swap
            .extra
            .as_mut()
            .unwrap()
            .insert("swap_fee_rate".to_owned(), "1".to_owned());
        assert!(validate_protocol_inputs(&item).is_ok());

        item.pool.all_tokens.truncate(1);
        assert!(validate_protocol_inputs(&item).is_err());
    }

    #[test]
    fn obric_requires_valid_price_object_ids_and_two_pool_tokens() {
        let mut item = sample_route_swap(vec![SUI, USDC]);
        item.pool.pool_type = SourceDex::Obric;
        item.swap.extra = Some(HashMap::from([
            ("x_price_id".to_owned(), "not-an-object-id".to_owned()),
            ("y_price_id".to_owned(), "0x2".to_owned()),
        ]));
        assert!(validate_protocol_inputs(&item).is_err());

        item.swap
            .extra
            .as_mut()
            .unwrap()
            .insert("x_price_id".to_owned(), "0x2".to_owned());
        assert!(validate_protocol_inputs(&item).is_ok());

        item.pool.all_tokens.truncate(1);
        assert!(validate_protocol_inputs(&item).is_err());
    }

    #[test]
    fn valid_swap_metadata_is_accepted() {
        assert!(validate_swap_inputs(&sample_swap(SUI, USDC, "0x2")).is_ok());
    }

    #[test]
    fn generic_swap_validation_does_not_require_protocol_metadata() {
        let mut swap = sample_swap(SUI, USDC, "0x2");
        swap.extra = None;
        assert!(validate_swap_inputs(&swap).is_ok());

        swap.extra = Some(HashMap::new());
        assert!(validate_swap_inputs(&swap).is_ok());
    }

    #[test]
    fn malformed_pool_tag_is_rejected_by_tag_parser() {
        let extra = HashMap::from([("pool_struct_tag".to_owned(), "not-a-struct-tag".to_owned())]);
        assert!(parse_pool_struct_tag(&extra).is_err());
    }

    #[test]
    fn invalid_input_and_output_types_are_rejected() {
        assert!(validate_swap_inputs(&sample_swap("not-a-type", USDC, "0x2")).is_err());
        assert!(validate_swap_inputs(&sample_swap(SUI, "not-a-type", "0x2")).is_err());
    }

    #[test]
    fn identical_assets_and_indexes_are_rejected() {
        assert!(validate_swap_inputs(&sample_swap(SUI, SUI, "0x2")).is_err());

        let mut swap = sample_swap(SUI, USDC, "0x2");
        swap.asset_out_index = swap.asset_in_index;
        assert!(validate_swap_inputs(&swap).is_err());
    }

    #[test]
    fn malformed_negative_and_overflowing_amounts_are_rejected() {
        let mut swap = sample_swap(SUI, USDC, "0x2");

        for amount in ["bad", "-1", "18446744073709551616"] {
            swap.amount = amount.to_owned();
            assert!(
                validate_swap_inputs(&swap).is_err(),
                "accepted amount: {amount}"
            );
        }

        swap.amount = "1".to_owned();
        swap.return_amount = "-1".to_owned();
        assert!(validate_swap_inputs(&swap).is_err());
    }

    #[test]
    fn zero_amounts_are_allowed() {
        let mut swap = sample_swap(SUI, USDC, "0x2");
        swap.amount = "0".to_owned();
        swap.return_amount = "0".to_owned();
        assert!(validate_swap_inputs(&swap).is_ok());
    }

    #[test]
    fn invalid_pool_id_is_rejected() {
        assert!(validate_swap_inputs(&sample_swap(SUI, USDC, "not-an-object-id")).is_err());
    }

    #[test]
    fn empty_route_is_rejected() {
        assert!(validate_route_inputs(&[]).is_err());
    }

    #[test]
    fn route_requires_both_assets_in_pool_metadata() {
        assert!(validate_route_inputs(&[sample_route_swap(vec![SUI, USDC])]).is_ok());

        assert!(validate_route_inputs(&[sample_route_swap(vec![SUI, SUI])]).is_err());
    }

    #[test]
    fn missing_pool_struct_tag_returns_error() {
        assert!(parse_pool_struct_tag(&HashMap::new()).is_err());
    }

    #[test]
    fn malformed_pool_struct_tag_returns_error() {
        let mut extra = HashMap::new();
        extra.insert("pool_struct_tag".to_owned(), "not-a-struct-tag".to_owned());
        assert!(parse_pool_struct_tag(&extra).is_err());
    }

    #[test]
    fn camel_case_pool_struct_tag_is_supported() {
        let mut extra = HashMap::new();
        extra.insert(
            "poolStructTag".to_owned(),
            "0x2::pool::Pool<0x2::sui::SUI,0x3::coin::COIN>".to_owned(),
        );
        assert!(parse_pool_struct_tag(&extra).is_ok());
    }
}
