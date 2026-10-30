#[cfg(test)]
use futures_util::{Stream, StreamExt};
#[cfg(test)]
use std::time::Duration;

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
    async fn treats_an_empty_stream_as_a_complete_empty_result() {
        let result = collect_stream(stream::empty::<Result<u8, &'static str>>()).await;

        assert_eq!(result, Ok(Vec::<u8>::new()));
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

    // Deliberately excluded from ordinary unit tests. CI invokes this ignored
    // test in a dedicated step after explicit user authorization.
    // It intentionally samples at most 1,000 objects and never claims completeness
    // when the cap is reached.
    #[tokio::test]
    #[ignore = "live read-only Sui testnet integration"]
    async fn live_testnet_returns_bounded_owned_sui_coin_sample() -> Result<(), Box<dyn std::error::Error>> {
        const MAX_SAMPLE: usize = 1000;

        let client = Client::new(Client::TESTNET_FULLNODE)?;
        let request = ListOwnedObjectsRequest::default()
            .with_owner(FIXTURE_OWNER)
            .with_object_type(SUI_COIN_TYPE);
        let stream = client.list_owned_objects(request);

        let (count, completed) = tokio::time::timeout(Duration::from_secs(30), async {
            futures_util::pin_mut!(stream);
            let mut count = 0usize;
            let mut completed = false;

            while count < MAX_SAMPLE {
                match stream.next().await {
                    Some(item) => {
                        // Any RPC error fails the probe; partial results are never
                        // represented as a complete inventory.
                        let _object = item?;
                        count += 1;
                    }
                    None => {
                        completed = true;
                        break;
                    }
                }
            }

            Ok::<(usize, bool), Box<dyn std::error::Error>>((count, completed))
        })
        .await??;

        println!(
            "LIVE_READ_ONLY_TESTNET_OK endpoint={} owner={} object_type={} sampled_objects={} completeness={}",
            Client::TESTNET_FULLNODE,
            FIXTURE_OWNER,
            SUI_COIN_TYPE,
            count,
            if completed { "complete_stream" } else { "bounded_sample_not_complete" }
        );
        Ok(())
    }
}
