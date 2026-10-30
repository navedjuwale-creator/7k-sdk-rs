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

    use std::sync::{Arc, Mutex};
    use tokio::sync::oneshot;
    use tonic::{Request, Response, Status};
    use sui_rpc::proto::sui::rpc::v2::{
        state_service_server::{StateService, StateServiceServer},
        ListOwnedObjectsResponse, Object,
    };

    #[derive(Clone, Copy)]
    enum PageScenario {
        TwoPages,
        EmptyMiddle,
        FailOnSecondPage,
        RepeatToken,
    }

    #[derive(Clone)]
    struct MockStateService {
        scenario: PageScenario,
        seen_tokens: Arc<Mutex<Vec<Option<Vec<u8>>>>>,
    }

    #[tonic::async_trait]
    impl StateService for MockStateService {
        async fn list_owned_objects(
            &self,
            request: Request<ListOwnedObjectsRequest>,
        ) -> Result<Response<ListOwnedObjectsResponse>, Status> {
            let token = request.into_inner().page_token.map(|value| value.to_vec());
            self.seen_tokens.lock().expect("request log lock").push(token.clone());

            if matches!(self.scenario, PageScenario::FailOnSecondPage)
                && token.as_deref() == Some(b"page-2")
            {
                return Err(Status::unavailable("simulated second-page failure"));
            }

            let mut response = ListOwnedObjectsResponse::default();
            match self.scenario {
                PageScenario::TwoPages | PageScenario::FailOnSecondPage if token.is_none() => {
                    let mut object = Object::default();
                    object.object_id = Some("fixture-object-1".to_owned());
                    response.objects.push(object);
                    response.next_page_token = Some(bytes::Bytes::from_static(b"page-2"));
                }
                PageScenario::TwoPages if token.as_deref() == Some(&b"page-2"[..]) => {
                    let mut object = Object::default();
                    object.object_id = Some("fixture-object-2".to_owned());
                    response.objects.push(object);
                }
                PageScenario::EmptyMiddle if token.is_none() => {
                    response.next_page_token = Some(bytes::Bytes::from_static(b"page-2"));
                }
                PageScenario::EmptyMiddle if token.as_deref() == Some(&b"page-2"[..]) => {
                    let mut object = Object::default();
                    object.object_id = Some("fixture-after-empty-page".to_owned());
                    response.objects.push(object);
                }
                PageScenario::RepeatToken if token.is_none() => {
                    let mut object = Object::default();
                    object.object_id = Some("fixture-repeat-1".to_owned());
                    response.objects.push(object);
                    response.next_page_token = Some(bytes::Bytes::from_static(b"repeat"));
                }
                PageScenario::RepeatToken if token.as_deref() == Some(&b"repeat"[..]) => {
                    let mut object = Object::default();
                    object.object_id = Some("fixture-repeat-2".to_owned());
                    response.objects.push(object);
                    response.next_page_token = Some(bytes::Bytes::from_static(b"repeat"));
                }
                _ => {}
            }
            Ok(Response::new(response))
        }
    }

    async fn start_mock_server(
        scenario: PageScenario,
    ) -> (String, Arc<Mutex<Vec<Option<Vec<u8>>>>>, oneshot::Sender<()>) {
        let seen_tokens = Arc::new(Mutex::new(Vec::new()));
        let service = MockStateService {
            scenario,
            seen_tokens: seen_tokens.clone(),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local mock gRPC server");
        let address = listener.local_addr().expect("local server address");
        drop(listener);

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(StateServiceServer::new(service))
                .serve_with_shutdown(address, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("mock gRPC server");
        });
        (format!("http://{address}"), seen_tokens, shutdown_tx)
    }

    fn fixture_request() -> ListOwnedObjectsRequest {
        ListOwnedObjectsRequest::default()
            .with_owner(FIXTURE_OWNER)
            .with_object_type(SUI_COIN_TYPE)
    }

    #[tokio::test]
    async fn actual_sdk_client_follows_two_pages_and_ends_naturally() {
        let (endpoint, seen, shutdown) = start_mock_server(PageScenario::TwoPages).await;
        let client = Client::new(endpoint).expect("local endpoint");
        let items = tokio::time::timeout(
            Duration::from_secs(3),
            collect_stream(client.list_owned_objects(fixture_request())),
        )
        .await
        .expect("pagination should finish")
        .expect("both pages should succeed");

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].object_id.as_deref(), Some("fixture-object-1"));
        assert_eq!(items[1].object_id.as_deref(), Some("fixture-object-2"));
        assert_eq!(
            *seen.lock().expect("request log lock"),
            vec![None, Some(b"page-2".to_vec())]
        );
        let _ = shutdown.send(());
    }

    #[tokio::test]
    async fn actual_sdk_client_follows_token_after_empty_intermediate_page() {
        let (endpoint, seen, shutdown) = start_mock_server(PageScenario::EmptyMiddle).await;
        let client = Client::new(endpoint).expect("local endpoint");
        let items = tokio::time::timeout(
            Duration::from_secs(3),
            collect_stream(client.list_owned_objects(fixture_request())),
        )
        .await
        .expect("empty intermediate page must not stall")
        .expect("later page should succeed");

        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].object_id.as_deref(),
            Some("fixture-after-empty-page")
        );
        assert_eq!(
            *seen.lock().expect("request log lock"),
            vec![None, Some(b"page-2".to_vec())]
        );
        let _ = shutdown.send(());
    }

    #[tokio::test]
    async fn actual_sdk_client_fails_closed_when_second_page_errors() {
        let (endpoint, seen, shutdown) = start_mock_server(PageScenario::FailOnSecondPage).await;
        let client = Client::new(endpoint).expect("local endpoint");
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            collect_stream(client.list_owned_objects(fixture_request())),
        )
        .await
        .expect("second-page error should be returned promptly");

        assert!(result.is_err(), "later-page RPC error must invalidate collection");
        assert_eq!(
            *seen.lock().expect("request log lock"),
            vec![None, Some(b"page-2".to_vec())]
        );
        let _ = shutdown.send(());
    }

    #[tokio::test]
    async fn repeated_continuation_token_is_detected_as_non_terminating_behavior() {
        let (endpoint, seen, shutdown) = start_mock_server(PageScenario::RepeatToken).await;
        let client = Client::new(endpoint).expect("local endpoint");
        let result = tokio::time::timeout(
            Duration::from_millis(250),
            collect_stream(client.list_owned_objects(fixture_request())),
        )
        .await;

        assert!(result.is_err(), "SDK unexpectedly terminated on repeated token");
        assert!(
            seen.lock().expect("request log lock").len() >= 2,
            "mock server should observe repeated-token pagination"
        );
        let _ = shutdown.send(());
    }

}
