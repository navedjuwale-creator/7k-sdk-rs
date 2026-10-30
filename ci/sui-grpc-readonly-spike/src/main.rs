use sui_rpc::proto::sui::rpc::v2::ListOwnedObjectsRequest;
use sui_rpc::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Client::new creates a lazy gRPC client; this proof does not make a network call.
    let client = Client::new(Client::TESTNET_FULLNODE)?;

    // Compile-check the read-only owned-object stream API without polling it.
    let request = ListOwnedObjectsRequest::default();
    let _owned_objects = client.list_owned_objects(request);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn exposes_read_only_owned_object_stream() {
        let client = Client::new(Client::TESTNET_FULLNODE).expect("valid endpoint");
        let request = ListOwnedObjectsRequest::default();
        let stream = client.list_owned_objects(request);
        drop(stream);
    }
}
