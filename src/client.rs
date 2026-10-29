use anyhow::Result;
use sui_sdk::{SuiClient, SuiClientBuilder};

#[derive(Debug, PartialEq, Eq)]
enum SuiNetwork {
    Localnet,
    Testnet,
    Devnet,
    Mainnet,
}

fn parse_network(value: &str) -> Result<SuiNetwork> {
    match value {
        "localnet" => Ok(SuiNetwork::Localnet),
        "testnet" => Ok(SuiNetwork::Testnet),
        "devnet" => Ok(SuiNetwork::Devnet),
        "mainnet" => Ok(SuiNetwork::Mainnet),
        _ => anyhow::bail!(
            "Unsupported SUI_NETWORK '{}'; expected localnet, testnet, devnet, or mainnet",
            value
        ),
    }
}

fn configured_network(value: Option<&str>) -> Result<SuiNetwork> {
    let value = value.ok_or_else(|| {
        anyhow::anyhow!("SUI_NETWORK is not set; set it to localnet, testnet, devnet, or mainnet")
    })?;
    parse_network(value)
}

pub async fn get_sui_client() -> Result<SuiClient> {
    let network_value = std::env::var("SUI_NETWORK");
    let network = match network_value {
        Ok(value) => configured_network(Some(&value))?,
        Err(std::env::VarError::NotPresent) => configured_network(None)?,
        Err(error) => anyhow::bail!("Could not read SUI_NETWORK: {error}"),
    };
    let builder = SuiClientBuilder::default();

    match network {
        SuiNetwork::Localnet => Ok(builder.build_localnet().await?),
        SuiNetwork::Testnet => Ok(builder.build_testnet().await?),
        SuiNetwork::Devnet => Ok(builder.build_devnet().await?),
        SuiNetwork::Mainnet => Ok(builder.build("https://fullnode.mainnet.sui.io:443").await?),
    }
}

#[cfg(test)]
mod tests {
    use super::{SuiNetwork, parse_network};

    #[test]
    fn supported_network_names_are_accepted() {
        assert_eq!(parse_network("localnet").unwrap(), SuiNetwork::Localnet);
        assert_eq!(parse_network("testnet").unwrap(), SuiNetwork::Testnet);
        assert_eq!(parse_network("devnet").unwrap(), SuiNetwork::Devnet);
        assert_eq!(parse_network("mainnet").unwrap(), SuiNetwork::Mainnet);
    }

    #[test]
    fn unsupported_network_names_are_rejected() {
        assert!(parse_network("").is_err());
        assert!(parse_network("production").is_err());
        assert!(parse_network("MAINNET").is_err());
    }

    #[test]
    fn missing_network_configuration_is_rejected() {
        let error = super::configured_network(None).unwrap_err();
        assert!(error.to_string().contains("SUI_NETWORK is not set"));
    }

    #[test]
    fn configured_network_names_are_validated() {
        assert_eq!(
            super::configured_network(Some("testnet")).unwrap(),
            SuiNetwork::Testnet
        );
        assert!(super::configured_network(Some("production")).is_err());
    }
}
