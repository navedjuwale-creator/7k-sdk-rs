#[cfg(test)]
use futures_util::{Stream, StreamExt};
use std::time::Duration;

use sui_rpc::proto::sui::rpc::v2::ListOwnedObjectsRequest;
use sui_rpc::proto::sui::rpc::v2::{
    state_service_client::StateServiceClient, Object,
};
use sui_rpc::Client;
use std::collections::HashSet;
use tonic::transport::Channel;

#[derive(Debug)]
enum PaginationError {
    Rpc(tonic::Status),
    RepeatedContinuationToken,
    PageLimitExceeded { max_pages: usize },
    DeadlineExceeded,
}

async fn collect_owned_objects_bounded<T>(
    client: &mut StateServiceClient<T>,
    mut request: ListOwnedObjectsRequest,
    max_pages: usize,
    max_duration: Duration,
) -> Result<Vec<Object>, PaginationError>
where
    T: tonic::client::GrpcService<tonic::body::Body>,
    T::Error: Into<tonic::codegen::StdError>,
    T::ResponseBody: tonic::codegen::Body<Data = tonic::codegen::Bytes> + Send + 'static,
    <T::ResponseBody as tonic::codegen::Body>::Error:
        Into<tonic::codegen::StdError> + Send,
{
    if max_pages == 0 {
        return Err(PaginationError::PageLimitExceeded { max_pages });
    }

    let deadline = tokio::time::Instant::now() + max_duration;
    let mut objects = Vec::new();
    let mut seen_tokens: HashSet<Vec<u8>> = HashSet::new();
    let mut pages_fetched = 0usize;

    loop {
        if let Some(token) = request.page_token.as_ref() {
            if !seen_tokens.insert(token.to_vec()) {
                return Err(PaginationError::RepeatedContinuationToken);
            }
        }

        pages_fetched += 1;
        let response = tokio::time::timeout_at(
            deadline,
            client.list_owned_objects(tonic::Request::new(request.clone())),
        )
        .await
        .map_err(|_| PaginationError::DeadlineExceeded)?
        .map_err(PaginationError::Rpc)?
        .into_inner();

        objects.extend(response.objects);
        match response.next_page_token {
            Some(token) => {
                if pages_fetched >= max_pages {
                    return Err(PaginationError::PageLimitExceeded { max_pages });
                }
                request.page_token = Some(token);
            }
            None => return Ok(objects),
        }
    }
}

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

    // Explicitly ignored: the workflow invokes this test only on the dedicated
    // test branch. Uses the SDK-configured transport and the bounded direct adapter.
    #[tokio::test]
    #[ignore = "live read-only Sui testnet bounded pagination integration"]
    async fn live_testnet_bounded_adapter_reaches_natural_end() -> Result<(), Box<dyn std::error::Error>> {
        const MAX_PAGES: usize = 8;
        let mut sdk_client = Client::new(Client::TESTNET_FULLNODE)?;
        let mut state_client = sdk_client.state_client();
        let request = fixture_request();

        let objects = collect_owned_objects_bounded(
            &mut state_client,
            request,
            MAX_PAGES,
            Duration::from_secs(30),
        )
        .await
        .map_err(|error| format!("bounded testnet pagination incomplete: {error:?}"))?;

        println!(
            "LIVE_BOUNDED_ADAPTER_OK endpoint={} owner={} object_type={} objects={} max_pages={} completeness=natural_end",
            Client::TESTNET_FULLNODE,
            FIXTURE_OWNER,
            SUI_COIN_TYPE,
            objects.len(),
            MAX_PAGES
        );
        Ok(())
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
        StallOnFirstPage,
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

            if matches!(self.scenario, PageScenario::StallOnFirstPage) {
                tokio::time::sleep(Duration::from_millis(500)).await;
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
        // Keep the already-bound listener and hand it to tonic. Dropping it
        // before the server task binds the same address creates a race where
        // clients can connect before the server is listening.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local mock gRPC server");
        let address = listener.local_addr().expect("local server address");
        let incoming = futures_util::stream::unfold(listener, |listener| async move {
            match listener.accept().await {
                Ok((stream, _)) => Some((Ok::<_, std::io::Error>(stream), listener)),
                Err(error) => Some((Err(error), listener)),
            }
        });

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(StateServiceServer::new(service))
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("mock gRPC server");
        });
        (format!("http://{address}"), seen_tokens, shutdown_tx)
    }

    fn fixture_request() -> ListOwnedObjectsRequest {
        // The RPC defaults to 50 objects per page. Request the documented
        // maximum (1,000) so the bounded live probe tests pagination rather
        // than exhausting its page budget on unnecessarily small pages.
        ListOwnedObjectsRequest::default()
            .with_owner(FIXTURE_OWNER)
            .with_object_type(SUI_COIN_TYPE)
            .with_page_size(1000)
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

    async fn direct_state_client(endpoint: String) -> StateServiceClient<Channel> {
        let channel = tonic::transport::Endpoint::from_shared(endpoint)
            .expect("valid mock endpoint")
            .connect()
            .await
            .expect("connect to mock endpoint");
        StateServiceClient::new(channel)
    }

    #[tokio::test]
    async fn bounded_adapter_detects_repeated_continuation_token() {
        let (endpoint, seen, shutdown) = start_mock_server(PageScenario::RepeatToken).await;
        let mut client = direct_state_client(endpoint).await;
        let result = collect_owned_objects_bounded(
            &mut client, fixture_request(), 10, Duration::from_secs(3),
        ).await;

        assert!(matches!(result, Err(PaginationError::RepeatedContinuationToken)));
        assert_eq!(
            *seen.lock().expect("request log lock"),
            vec![None, Some(b"repeat".to_vec())]
        );
        let _ = shutdown.send(());
    }

    #[tokio::test]
    async fn bounded_adapter_collects_pages_and_stops_at_end_token() {
        let (endpoint, seen, shutdown) = start_mock_server(PageScenario::TwoPages).await;
        let mut client = direct_state_client(endpoint).await;
        let objects = collect_owned_objects_bounded(
            &mut client, fixture_request(), 2, Duration::from_secs(3),
        )
            .await
            .expect("two pages should complete");

        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].object_id.as_deref(), Some("fixture-object-1"));
        assert_eq!(objects[1].object_id.as_deref(), Some("fixture-object-2"));
        assert_eq!(
            *seen.lock().expect("request log lock"),
            vec![None, Some(b"page-2".to_vec())]
        );
        let _ = shutdown.send(());
    }

    #[tokio::test]
    async fn bounded_adapter_fails_closed_when_page_budget_is_exhausted() {
        let (endpoint, seen, shutdown) = start_mock_server(PageScenario::TwoPages).await;
        let mut client = direct_state_client(endpoint).await;
        let result = collect_owned_objects_bounded(
            &mut client, fixture_request(), 1, Duration::from_secs(3),
        ).await;

        assert!(matches!(
            result,
            Err(PaginationError::PageLimitExceeded { max_pages: 1 })
        ));
        assert_eq!(seen.lock().expect("request log lock").len(), 1);
        let _ = shutdown.send(());
    }

    #[tokio::test]
    async fn bounded_adapter_enforces_deadline_on_stalled_rpc() {
        let (endpoint, _seen, shutdown) = start_mock_server(PageScenario::StallOnFirstPage).await;
        let mut client = direct_state_client(endpoint).await;
        let started = tokio::time::Instant::now();
        let result = collect_owned_objects_bounded(
            &mut client, fixture_request(), 10, Duration::from_millis(50),
        ).await;
        let elapsed = started.elapsed();

        assert!(matches!(result, Err(PaginationError::DeadlineExceeded)));
        assert!(elapsed < Duration::from_millis(250), "deadline was not enforced promptly: {elapsed:?}");
        let _ = shutdown.send(());
    }

    #[tokio::test]
    async fn bounded_adapter_fails_closed_on_later_page_rpc_error() {
        let (endpoint, seen, shutdown) =
            start_mock_server(PageScenario::FailOnSecondPage).await;
        let mut client = direct_state_client(endpoint).await;
        let result = collect_owned_objects_bounded(
            &mut client, fixture_request(), 10, Duration::from_secs(3),
        ).await;

        assert!(matches!(result, Err(PaginationError::Rpc(_))));
        assert_eq!(seen.lock().expect("request log lock").len(), 2);
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
