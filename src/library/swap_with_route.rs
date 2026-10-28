use std::str::FromStr;

use anyhow::Result;
use move_core_types::language_storage::StructTag;
use sui_sdk::{
    types::{transaction::Argument, TypeTag},
    SuiClient,
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
        .get("pool_struct_tag")
        .ok_or_else(|| anyhow::anyhow!("Missing pool_struct_tag in swap metadata"))?;

    StructTag::from_str(pool_struct_tag).map_err(Into::into)
}

fn validate_swap_inputs(swap: &SorSwap) -> Result<()> {
    TypeTag::from_str(&swap.asset_in)
        .map_err(|e| anyhow::anyhow!("Invalid input asset type: {e}"))?;
    TypeTag::from_str(&swap.asset_out)
        .map_err(|e| anyhow::anyhow!("Invalid output asset type: {e}"))?;
    sui_sdk::types::base_types::ObjectID::from_str(&swap.pool_id)
        .map_err(|e| anyhow::anyhow!("Invalid pool ID: {e}"))?;
    Ok(())
}

fn validate_route_inputs(route: &[TxSorSwap]) -> Result<()> {
    if route.is_empty() {
        anyhow::bail!("Swap route must contain at least one hop");
    }

    for (index, item) in route.iter().enumerate() {
        validate_swap_inputs(&item.swap)
            .map_err(|e| anyhow::anyhow!("Invalid swap at route hop {}: {e}", index + 1))?;
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
    use super::{parse_pool_struct_tag, validate_route_inputs, validate_swap_inputs};
    use crate::types::aggregators::SorSwap;
    use std::collections::HashMap;

    fn sample_swap(asset_in: &str, asset_out: &str, pool_id: &str) -> SorSwap {
        SorSwap {
            pool_id: pool_id.to_owned(),
            asset_in_index: 0,
            asset_out_index: 1,
            amount: "1".to_owned(),
            return_amount: "1".to_owned(),
            asset_in: asset_in.to_owned(),
            asset_out: asset_out.to_owned(),
            function_name: String::new(),
            arguments: Vec::new(),
            extra: None,
        }
    }

    #[test]
    fn valid_swap_metadata_is_accepted() {
        let swap = sample_swap("0x2::sui::SUI", "0x2::sui::SUI", "0x2");
        assert!(validate_swap_inputs(&swap).is_ok());
    }

    #[test]
    fn invalid_input_type_is_rejected() {
        let swap = sample_swap("not-a-type", "0x2::sui::SUI", "0x2");
        assert!(validate_swap_inputs(&swap).is_err());
    }

    #[test]
    fn invalid_output_type_is_rejected() {
        let swap = sample_swap("0x2::sui::SUI", "not-a-type", "0x2");
        assert!(validate_swap_inputs(&swap).is_err());
    }

    #[test]
    fn invalid_pool_id_is_rejected() {
        let swap = sample_swap("0x2::sui::SUI", "0x2::sui::SUI", "not-an-object-id");
        assert!(validate_swap_inputs(&swap).is_err());
    }

    #[test]
    fn empty_route_is_rejected() {
        assert!(validate_route_inputs(&[]).is_err());
    }

    #[test]
    fn missing_pool_struct_tag_returns_error() {
        let extra = HashMap::new();
        assert!(parse_pool_struct_tag(&extra).is_err());
    }

    #[test]
    fn malformed_pool_struct_tag_returns_error() {
        let mut extra = HashMap::new();
        extra.insert("pool_struct_tag".to_owned(), "not-a-struct-tag".to_owned());
        assert!(parse_pool_struct_tag(&extra).is_err());
    }
}
