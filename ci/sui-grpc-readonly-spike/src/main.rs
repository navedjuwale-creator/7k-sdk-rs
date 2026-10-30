use futures_util::{Stream, StreamExt};
use sui_rpc::proto::sui::rpc::v2::ListOwnedObjectsRequest;
use sui_rpc::Client;

const FIXTURE_OWNER: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000000";
const SUI_COIN_TYPE: &str = "0x2::coin::Coin<0x2::sui::SUI>";

#[cfg(test)]
async fn collect_stream<S, T, E>(stream: S) -> Result<Vec<T>, E>
where
    S: Stream<Item = Result<T, E>>,
{
    futures_util::pin_mut!(stream);
    let mut items = Vec::new();

    while let Some(item) = stream.next().await {
        // Fail closed: an RPC error must not be mistaken for a complete result.
        items.push(item?);
    }

    Ok(items)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Client::new creates a lazy gRPC client; this proof does not make a network call.
    let client = Client::new(Client::TESTNET_FULLNODE)?;

    // Compile-check a scoped, read-only owned-SUI-coin listing request.
    let request = ListOwnedObjectsRequest::default()
        .with_owner(FIXTURE_OWNER)
        .with_object_type(SUI_COIN_TYPE);
    let _owned_objects = client.list_owned_objects(request);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    #[tokio::test]
    async fn constructs_testnet_client_without_network_io() {
        let client = Client::new(Client::TESTNET_FULLNODE);
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn rejects_malformed_endpoint_without_network_io() {
        assert!(Client::new("not a valid endpoint").is_err());
    }

    #[tokio::test]
    async fn builds_owner_and_coin_type_filtered_request() {
        let request = ListOwnedObjectsRequest::default()
            .with_owner(FIXTURE_OWNER)
            .with_object_type(SUI_COIN_TYPE);

        assert_eq!(request.owner_opt(), Some(FIXTURE_OWNER));
        assert_eq!(request.object_type_opt(), Some(SUI_COIN_TYPE));
    }

    #[tokio::test]
    async fn exposes_read_only_owned_object_stream_without_polling_network() {
        let client = Client::new(Client::TESTNET_FULLNODE).expect("valid endpoint");
        let request = ListOwnedObjectsRequest::default()
            .with_owner(FIXTURE_OWNER)
            .with_object_type(SUI_COIN_TYPE);
        let stream = client.list_owned_objects(request);
        drop(stream);
    }

    #[tokio::test]
    async fn collects_every_successful_stream_item() {
        let result = collect_stream(stream::iter([
            Ok::<_, &'static str>(10),
            Ok(20),
            Ok(30),
        ]))
        .await;

        assert_eq!(result, Ok(vec![10, 20, 30]));
    }

    #[tokio::test]
    async fn propagates_stream_error_instead_of_returning_partial_results() {
        let result = collect_stream(stream::iter([
            Ok::<_, &'static str>(10),
            Err("simulated RPC failure"),
            Ok(30),
        ]))
        .await;

        assert_eq!(result, Err("simulated RPC failure"));
    }
}
