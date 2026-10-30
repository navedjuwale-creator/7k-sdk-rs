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

fn ensure_network_supported(network: &SuiNetwork) -> Result<()> {
    if *network == SuiNetwork::Mainnet {
        anyhow::bail!(
            "Sui mainnet is temporarily disabled until this SDK uses a supported \
             gRPC or GraphQL transport."
        );
    }
    Ok(())
}

fn select_network(value: Option<&str>) -> Result<SuiNetwork> {
    let network = match value {
        Some(value) => parse_network(value)?,
        None => SuiNetwork::Localnet,
    };
    ensure_network_supported(&network)?;
    Ok(network)
}

fn network_from_env(
    read_env: impl FnOnce() -> std::result::Result<String, std::env::VarError>,
) -> Result<SuiNetwork> {
    let network_value = match read_env() {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => anyhow::bail!("Could not read SUI_NETWORK: {error}"),
    };

    select_network(network_value.as_deref())
}

pub async fn get_sui_client() -> Result<SuiClient> {
    let network = network_from_env(|| std::env::var("SUI_NETWORK"))?;
    let builder = SuiClientBuilder::default();

    match network {
        SuiNetwork::Localnet => Ok(builder.build_localnet().await?),
        SuiNetwork::Testnet => Ok(builder.build_testnet().await?),
        SuiNetwork::Devnet => Ok(builder.build_devnet().await?),
        SuiNetwork::Mainnet => anyhow::bail!(
            "Sui mainnet is temporarily disabled until this SDK uses a supported gRPC or GraphQL transport."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{network_from_env, parse_network, select_network, SuiNetwork};

    #[test]
    fn environment_lookup_defaults_to_localnet_when_variable_is_missing() {
        let network = network_from_env(|| Err(std::env::VarError::NotPresent)).unwrap();
        assert_eq!(network, SuiNetwork::Localnet);
    }

    #[test]
    fn environment_lookup_selects_the_configured_network() {
        let network =
            network_from_env(|| Ok("testnet".to_owned())).expect("testnet should be selectable");
        assert_eq!(network, SuiNetwork::Testnet);
    }

    #[test]
    fn environment_lookup_rejects_disabled_mainnet() {
        let error = network_from_env(|| Ok("mainnet".to_owned()))
            .expect_err("mainnet must remain disabled");
        assert!(error.to_string().contains("temporarily disabled"));
    }

    #[test]
    fn environment_lookup_rejects_invalid_values() {
        assert!(network_from_env(|| Ok("production".to_owned())).is_err());
    }

    #[test]
    fn environment_lookup_reports_non_unicode_values() {
        let error = network_from_env(|| {
            Err(std::env::VarError::NotUnicode(std::ffi::OsString::from(
                "invalid",
            )))
        })
        .expect_err("non-Unicode environment values must be reported");
        assert!(error.to_string().contains("Could not read SUI_NETWORK"));
    }

    #[test]
    fn missing_network_defaults_to_localnet() {
        assert_eq!(select_network(None).unwrap(), SuiNetwork::Localnet);
    }

    #[test]
    fn supported_non_mainnet_networks_are_selectable() {
        assert_eq!(
            select_network(Some("localnet")).unwrap(),
            SuiNetwork::Localnet
        );
        assert_eq!(
            select_network(Some("testnet")).unwrap(),
            SuiNetwork::Testnet
        );
        assert_eq!(select_network(Some("devnet")).unwrap(), SuiNetwork::Devnet);
    }

    #[test]
    fn mainnet_is_rejected_during_selection() {
        assert!(select_network(Some("mainnet")).is_err());
    }

    #[test]
    fn invalid_network_is_rejected_during_selection() {
        assert!(select_network(Some("production")).is_err());
        assert!(select_network(Some("MAINNET")).is_err());
    }

    #[test]
    fn supported_network_names_are_accepted() {
        assert_eq!(parse_network("localnet").unwrap(), SuiNetwork::Localnet);
        assert_eq!(parse_network("testnet").unwrap(), SuiNetwork::Testnet);
        assert_eq!(parse_network("devnet").unwrap(), SuiNetwork::Devnet);
        assert_eq!(parse_network("mainnet").unwrap(), SuiNetwork::Mainnet);
    }

    #[test]
    fn mainnet_is_rejected_by_network_guard() {
        let error = super::ensure_network_supported(&SuiNetwork::Mainnet)
            .expect_err("mainnet must remain disabled");
        assert!(error.to_string().contains("temporarily disabled"));
    }

    #[test]
    fn non_mainnet_networks_are_allowed_by_the_guard() {
        assert!(super::ensure_network_supported(&SuiNetwork::Localnet).is_ok());
        assert!(super::ensure_network_supported(&SuiNetwork::Testnet).is_ok());
        assert!(super::ensure_network_supported(&SuiNetwork::Devnet).is_ok());
    }

    #[test]
    fn unsupported_network_names_are_rejected() {
        assert!(parse_network("").is_err());
        assert!(parse_network("production").is_err());
        assert!(parse_network("MAINNET").is_err());
    }
}
