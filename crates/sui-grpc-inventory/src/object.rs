use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use prost_types::FieldMask;
use std::time::Duration;
use sui_rpc::{
    Client,
    proto::sui::rpc::v2::{
        GetObjectRequest, GetObjectResponse, GetServiceInfoRequest, GetServiceInfoResponse, Object,
    },
};

const RPC_DEADLINE: Duration = Duration::from_secs(10);
const OBJECT_READ_MASK: &[&str] = &["object_id", "version", "digest", "owner", "object_type"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiGrpcNetwork {
    pub endpoint: String,
    pub expected_chain_id: String,
}

impl SuiGrpcNetwork {
    pub fn new(endpoint: impl Into<String>, expected_chain_id: impl Into<String>) -> Result<Self> {
        let endpoint = endpoint.into();
        let expected_chain_id = expected_chain_id.into();
        if !endpoint.starts_with("https://") {
            bail!("Sui gRPC endpoint must use HTTPS");
        }
        if expected_chain_id.trim().is_empty() || expected_chain_id.trim() != expected_chain_id {
            bail!("Expected Sui chain ID must be non-empty and contain no surrounding whitespace");
        }
        Ok(Self {
            endpoint,
            expected_chain_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuiOwner {
    Address { address: String },
    Object { object_id: String },
    Shared { initial_shared_version: u64 },
    Immutable,
    ConsensusAddress { address: String, start_version: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiObjectRecord {
    pub chain_id: String,
    pub object_id: String,
    pub version: u64,
    pub digest: String,
    pub object_type: String,
    pub owner: SuiOwner,
    pub contents_bcs: Option<Vec<u8>>,
}

#[async_trait]
trait ObjectReadSource: Send {
    async fn get_service_info(&mut self) -> Result<GetServiceInfoResponse>;
    async fn get_object(&mut self, request: GetObjectRequest) -> Result<GetObjectResponse>;
}

struct SuiRpcObjectReadSource {
    client: Client,
}

impl SuiRpcObjectReadSource {
    fn new(endpoint: &str) -> Result<Self> {
        let client = Client::new(endpoint).context("Failed to construct Sui gRPC client")?;
        Ok(Self { client })
    }
}

#[async_trait]
impl ObjectReadSource for SuiRpcObjectReadSource {
    async fn get_service_info(&mut self) -> Result<GetServiceInfoResponse> {
        self.client
            .ledger_client()
            .get_service_info(GetServiceInfoRequest::default())
            .await
            .map(|response| response.into_inner())
            .context("Sui gRPC GetServiceInfo request failed")
    }

    async fn get_object(&mut self, request: GetObjectRequest) -> Result<GetObjectResponse> {
        self.client
            .ledger_client()
            .get_object(request)
            .await
            .map(|response| response.into_inner())
            .context("Sui gRPC GetObject request failed")
    }
}

/// Read-only object reader bound to one verified Sui chain identity.
/// It deliberately exposes no transaction-builder or signing interfaces.
pub struct SuiGrpcObjectReader {
    source: Box<dyn ObjectReadSource>,
    network: SuiGrpcNetwork,
}

impl SuiGrpcObjectReader {
    pub async fn connect(network: SuiGrpcNetwork) -> Result<Self> {
        let source = SuiRpcObjectReadSource::new(&network.endpoint)?;
        Self::connect_with_source(network, Box::new(source)).await
    }

    async fn connect_with_source(
        network: SuiGrpcNetwork,
        mut source: Box<dyn ObjectReadSource>,
    ) -> Result<Self> {
        let info = tokio::time::timeout(RPC_DEADLINE, source.get_service_info())
            .await
            .map_err(|_| anyhow!("Sui gRPC GetServiceInfo deadline exceeded"))??;
        let observed_chain_id = info
            .chain_id
            .context("Sui gRPC service info is missing chain_id")?;
        if observed_chain_id.trim().is_empty() || observed_chain_id.trim() != observed_chain_id {
            bail!("Sui gRPC service returned an invalid chain_id");
        }
        if observed_chain_id != network.expected_chain_id {
            bail!(
                "Sui gRPC chain identity mismatch: configured expected chain ID does not match endpoint"
            );
        }
        Ok(Self { source, network })
    }

    pub fn chain_id(&self) -> &str {
        &self.network.expected_chain_id
    }

    /// Fetch the latest object version. BCS contents are opt-in and omitted by default.
    pub async fn get_object(&mut self, object_id: &str) -> Result<SuiObjectRecord> {
        self.get_object_with_contents(object_id, false).await
    }

    /// Fetch an object, optionally including raw BCS Move contents.
    pub async fn get_object_with_contents(
        &mut self,
        object_id: &str,
        include_contents: bool,
    ) -> Result<SuiObjectRecord> {
        validate_object_id(object_id)?;
        let canonical_requested_id = canonical_hex_id(object_id)?;
        let mut paths: Vec<String> = OBJECT_READ_MASK
            .iter()
            .map(|path| (*path).to_owned())
            .collect();
        if include_contents {
            paths.push("contents".to_owned());
        }
        let mut request = GetObjectRequest::default();
        request.object_id = Some(object_id.to_owned());
        request.read_mask = Some(FieldMask { paths });
        let response = tokio::time::timeout(RPC_DEADLINE, self.source.get_object(request))
            .await
            .map_err(|_| anyhow!("Sui gRPC GetObject deadline exceeded"))??;
        let object = response
            .object
            .context("Sui gRPC GetObject response is missing object")?;
        decode_object(
            object,
            &self.network.expected_chain_id,
            &canonical_requested_id,
            include_contents,
        )
    }
}

fn decode_object(
    object: Object,
    chain_id: &str,
    requested_object_id: &str,
    include_contents: bool,
) -> Result<SuiObjectRecord> {
    let object_id = object
        .object_id
        .context("Sui gRPC object is missing object_id")?;
    validate_object_id(&object_id)?;
    if canonical_hex_id(&object_id)? != requested_object_id {
        bail!("Sui gRPC returned an object ID different from the requested ID");
    }

    let version = object
        .version
        .context("Sui gRPC object is missing version")?;
    if version == 0 {
        bail!("Sui gRPC object has invalid version zero");
    }

    let digest = object.digest.context("Sui gRPC object is missing digest")?;
    if digest.trim().is_empty() || digest.trim() != digest {
        bail!("Sui gRPC object has an invalid digest");
    }

    let object_type = object
        .object_type
        .context("Sui gRPC object is missing object_type")?;
    if object_type.trim().is_empty() || object_type.trim() != object_type {
        bail!("Sui gRPC object has an invalid object_type");
    }

    let owner = object.owner.context("Sui gRPC object is missing owner")?;
    let owner = decode_owner(owner)?;

    let contents_bcs = if include_contents {
        Some(
            object
                .contents
                .context("Sui gRPC object is missing requested contents")?
                .value
                .context("Sui gRPC object contents is missing BCS value")?
                .to_vec(),
        )
    } else {
        None
    };

    Ok(SuiObjectRecord {
        chain_id: chain_id.to_owned(),
        object_id,
        version,
        digest,
        object_type,
        owner,
        contents_bcs,
    })
}

fn decode_owner(owner: sui_rpc::proto::sui::rpc::v2::Owner) -> Result<SuiOwner> {
    let kind = owner.kind.context("Sui object owner is missing kind")?;
    match kind {
        1 => {
            let address = owner.address.context("Address owner is missing address")?;
            validate_object_id(&address).context("Address owner has invalid address")?;
            if owner.version.is_some() {
                bail!("Address owner unexpectedly contains a version");
            }
            Ok(SuiOwner::Address { address })
        }
        2 => {
            let object_id = owner.address.context("Object owner is missing object ID")?;
            validate_object_id(&object_id).context("Object owner has invalid object ID")?;
            if owner.version.is_some() {
                bail!("Object owner unexpectedly contains a version");
            }
            Ok(SuiOwner::Object { object_id })
        }
        3 => {
            let initial_shared_version = owner
                .version
                .context("Shared owner is missing initial shared version")?;
            if initial_shared_version == 0 || owner.address.is_some() {
                bail!("Shared owner contains invalid or conflicting metadata");
            }
            Ok(SuiOwner::Shared {
                initial_shared_version,
            })
        }
        4 => {
            if owner.address.is_some() || owner.version.is_some() {
                bail!("Immutable owner unexpectedly contains address or version metadata");
            }
            Ok(SuiOwner::Immutable)
        }
        5 => {
            let address = owner
                .address
                .context("Consensus-address owner is missing address")?;
            validate_object_id(&address).context("Consensus-address owner has invalid address")?;
            let start_version = owner
                .version
                .context("Consensus-address owner is missing start version")?;
            if start_version == 0 {
                bail!("Consensus-address owner has invalid start version zero");
            }
            Ok(SuiOwner::ConsensusAddress {
                address,
                start_version,
            })
        }
        other => bail!("Sui object owner has unsupported kind {other}"),
    }
}

fn validate_object_id(object_id: &str) -> Result<()> {
    let value = object_id.strip_prefix("0x").unwrap_or(object_id);
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("Sui object ID/address must be a 32-byte hexadecimal value");
    }
    Ok(())
}

fn canonical_hex_id(value: &str) -> Result<String> {
    validate_object_id(value)?;
    Ok(value
        .strip_prefix("0x")
        .unwrap_or(value)
        .to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::{ObjectReadSource, SuiGrpcNetwork, SuiGrpcObjectReader, SuiOwner, decode_object};
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use sui_rpc::proto::sui::rpc::v2::{
        GetObjectRequest, GetObjectResponse, GetServiceInfoResponse, Object, Owner,
    };

    const CHAIN: &str = "test-chain-genesis-digest";
    const OBJECT_ID: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ADDRESS: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    struct MockSource {
        info: Option<Result<GetServiceInfoResponse>>,
        objects: VecDeque<Result<GetObjectResponse>>,
        last_request: Option<GetObjectRequest>,
    }

    #[async_trait]
    impl ObjectReadSource for MockSource {
        async fn get_service_info(&mut self) -> Result<GetServiceInfoResponse> {
            self.info
                .take()
                .unwrap_or_else(|| Err(anyhow!("service info requested more than once")))
        }

        async fn get_object(&mut self, request: GetObjectRequest) -> Result<GetObjectResponse> {
            self.last_request = Some(request);
            self.objects
                .pop_front()
                .unwrap_or_else(|| Err(anyhow!("unexpected object request")))
        }
    }

    fn network(expected: &str) -> SuiGrpcNetwork {
        SuiGrpcNetwork::new("https://sui.example.invalid:443", expected).unwrap()
    }

    fn service_info(chain_id: Option<&str>) -> GetServiceInfoResponse {
        let mut info = GetServiceInfoResponse::default();
        info.chain_id = chain_id.map(str::to_owned);
        info
    }

    fn owner(kind: i32, address: Option<&str>, version: Option<u64>) -> Owner {
        let mut owner = Owner::default();
        owner.kind = Some(kind);
        owner.address = address.map(str::to_owned);
        owner.version = version;
        owner
    }

    fn object(owner: Owner) -> Object {
        let mut object = Object::default();
        object.object_id = Some(OBJECT_ID.to_owned());
        object.version = Some(7);
        object.digest = Some("valid-digest".to_owned());
        object.owner = Some(owner);
        object.object_type = Some("0x2::example::Thing".to_owned());
        object
    }

    struct RecordingSource {
        response: Option<GetObjectResponse>,
        captured: std::sync::Arc<std::sync::Mutex<Option<GetObjectRequest>>>,
    }

    #[async_trait]
    impl ObjectReadSource for RecordingSource {
        async fn get_service_info(&mut self) -> Result<GetServiceInfoResponse> {
            Ok(service_info(Some(CHAIN)))
        }

        async fn get_object(&mut self, request: GetObjectRequest) -> Result<GetObjectResponse> {
            *self.captured.lock().unwrap() = Some(request);
            self.response
                .take()
                .ok_or_else(|| anyhow!("unexpected second object request"))
        }
    }

    #[tokio::test]
    async fn public_get_object_request_uses_exact_required_field_mask() {
        let mut response = GetObjectResponse::default();
        response.object = Some(object(owner(4, None, None)));
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let source = RecordingSource {
            response: Some(response),
            captured: captured.clone(),
        };
        let mut reader = SuiGrpcObjectReader {
            source: Box::new(source),
            network: network(CHAIN),
        };

        let record = reader.get_object(OBJECT_ID).await.unwrap();
        assert_eq!(record.object_id, OBJECT_ID);

        let request = captured.lock().unwrap().clone().unwrap();
        assert_eq!(request.object_id.as_deref(), Some(OBJECT_ID));
        let paths = request.read_mask.unwrap().paths;
        assert_eq!(
            paths,
            vec!["object_id", "version", "digest", "owner", "object_type"]
        );
        assert!(!paths.iter().any(|path| path == "contents"));
    }

    #[tokio::test]
    async fn public_get_object_with_contents_adds_only_contents_to_mask() {
        let mut response = GetObjectResponse::default();
        let mut value = sui_rpc::proto::sui::rpc::v2::Bcs::default();
        value.value = Some(vec![1, 2, 3].into());
        let mut object = object(owner(4, None, None));
        object.contents = Some(value);
        response.object = Some(object);
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let source = RecordingSource {
            response: Some(response),
            captured: captured.clone(),
        };
        let mut reader = SuiGrpcObjectReader {
            source: Box::new(source),
            network: network(CHAIN),
        };

        let record = reader
            .get_object_with_contents(OBJECT_ID, true)
            .await
            .unwrap();
        assert_eq!(record.contents_bcs, Some(vec![1, 2, 3]));

        let request = captured.lock().unwrap().clone().unwrap();
        let paths = request.read_mask.unwrap().paths;
        assert_eq!(
            paths,
            vec![
                "object_id",
                "version",
                "digest",
                "owner",
                "object_type",
                "contents"
            ]
        );
    }

    /// Diagnostic probe used once to capture the live full Testnet chain ID.
    #[tokio::test]
    #[ignore = "requires public Sui Testnet gRPC connectivity"]
    async fn live_testnet_probe_reports_chain_identity() {
        let client = sui_rpc::Client::new("https://fullnode.testnet.sui.io:443").unwrap();
        let info = client
            .ledger_client()
            .get_service_info(GetServiceInfoRequest::default())
            .await
            .unwrap()
            .into_inner();
        let chain = info.chain.as_deref().unwrap_or("");
        let chain_id = info.chain_id.as_deref().unwrap_or("");
        assert_eq!(
            chain, "testnet",
            "endpoint did not identify itself as Sui Testnet"
        );
        assert!(!chain_id.is_empty(), "GetServiceInfo omitted chain_id");
        eprintln!("SUI_TESTNET_CHAIN_ID={chain_id}");
    }

    /// Live smoke test. Run explicitly with SUI_TESTNET_EXPECTED_CHAIN_ID set to
    /// the independently pinned full Base58 genesis checkpoint digest.
    #[tokio::test]
    #[ignore = "requires public Sui Testnet gRPC connectivity and pinned chain ID"]
    async fn live_testnet_chain_identity_and_read_only_object_lookup() {
        let endpoint = "https://fullnode.testnet.sui.io:443";
        let expected = std::env::var("SUI_TESTNET_EXPECTED_CHAIN_ID")
            .expect("SUI_TESTNET_EXPECTED_CHAIN_ID must be explicitly configured");
        let mut reader =
            SuiGrpcObjectReader::connect(SuiGrpcNetwork::new(endpoint, expected.clone()).unwrap())
                .await
                .unwrap();
        assert_eq!(reader.chain_id(), expected);
        let record = reader
            .get_object("0x0000000000000000000000000000000000000000000000000000000000000005")
            .await
            .unwrap();
        assert_eq!(record.chain_id, expected);
        assert_eq!(
            record.object_id.to_ascii_lowercase(),
            "0x0000000000000000000000000000000000000000000000000000000000000005"
        );
        eprintln!(
            "LIVE_SUI_TESTNET_OK chain_id={} object_version={} object_type={}",
            record.chain_id, record.version, record.object_type
        );
    }

    fn reader_with_object(object: Object) -> SuiGrpcObjectReader {
        let mut response = GetObjectResponse::default();
        response.object = Some(object);
        let mock = MockSource {
            info: Some(Ok(service_info(Some(CHAIN)))),
            objects: VecDeque::from([Ok(response)]),
            last_request: None,
        };
        // Construct the reader directly so tests exercise the object adapter
        // without making network requests.
        SuiGrpcObjectReader {
            source: Box::new(mock),
            network: network(CHAIN),
        }
    }

    #[tokio::test]
    async fn verifies_chain_identity_before_returning_reader() {
        let mock = MockSource {
            info: Some(Ok(service_info(Some(CHAIN)))),
            objects: VecDeque::new(),
            last_request: None,
        };
        assert!(
            SuiGrpcObjectReader::connect_with_source(network(CHAIN), Box::new(mock))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn rejects_chain_id_mismatch() {
        let mock = MockSource {
            info: Some(Ok(service_info(Some("different-chain")))),
            objects: VecDeque::new(),
            last_request: None,
        };
        assert!(
            SuiGrpcObjectReader::connect_with_source(network(CHAIN), Box::new(mock))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn rejects_missing_chain_id() {
        let mock = MockSource {
            info: Some(Ok(service_info(None))),
            objects: VecDeque::new(),
            last_request: None,
        };
        assert!(
            SuiGrpcObjectReader::connect_with_source(network(CHAIN), Box::new(mock))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn rejects_service_info_rpc_error() {
        let mock = MockSource {
            info: Some(Err(anyhow!("transport failure"))),
            objects: VecDeque::new(),
            last_request: None,
        };
        assert!(
            SuiGrpcObjectReader::connect_with_source(network(CHAIN), Box::new(mock))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn decodes_address_owned_object() {
        let record = decode_object(
            object(owner(1, Some(ADDRESS), None)),
            CHAIN,
            &"a".repeat(64),
            false,
        )
        .unwrap();
        assert_eq!(
            record.owner,
            SuiOwner::Address {
                address: ADDRESS.to_owned()
            }
        );
    }

    #[tokio::test]
    async fn decodes_object_owned_object() {
        let record = decode_object(
            object(owner(2, Some(ADDRESS), None)),
            CHAIN,
            &"a".repeat(64),
            false,
        )
        .unwrap();
        assert_eq!(
            record.owner,
            SuiOwner::Object {
                object_id: ADDRESS.to_owned()
            }
        );
    }

    #[tokio::test]
    async fn preserves_exact_shared_initial_version() {
        let record = decode_object(
            object(owner(3, None, Some(42))),
            CHAIN,
            &"a".repeat(64),
            false,
        )
        .unwrap();
        assert_eq!(
            record.owner,
            SuiOwner::Shared {
                initial_shared_version: 42
            }
        );
    }

    #[tokio::test]
    async fn decodes_immutable_owner() {
        let record =
            decode_object(object(owner(4, None, None)), CHAIN, &"a".repeat(64), false).unwrap();
        assert_eq!(record.owner, SuiOwner::Immutable);
    }

    #[tokio::test]
    async fn preserves_consensus_address_and_start_version() {
        let record = decode_object(
            object(owner(5, Some(ADDRESS), Some(19))),
            CHAIN,
            &"a".repeat(64),
            false,
        )
        .unwrap();
        assert_eq!(
            record.owner,
            SuiOwner::ConsensusAddress {
                address: ADDRESS.to_owned(),
                start_version: 19
            }
        );
    }

    #[test]
    fn rejects_owner_with_missing_or_conflicting_metadata() {
        assert!(super::decode_owner(owner(3, None, None)).is_err());
        assert!(super::decode_owner(owner(4, Some(ADDRESS), None)).is_err());
        assert!(super::decode_owner(owner(1, Some(ADDRESS), Some(2))).is_err());
        assert!(super::decode_owner(owner(5, Some(ADDRESS), Some(0))).is_err());
        assert!(super::decode_owner(owner(99, None, None)).is_err());
    }

    #[test]
    fn rejects_missing_object_fields_and_wrong_object_id() {
        assert!(decode_object(Object::default(), CHAIN, &"a".repeat(64), false).is_err());
        let mut missing_owner = object(owner(4, None, None));
        missing_owner.owner = None;
        assert!(decode_object(missing_owner, CHAIN, &"a".repeat(64), false).is_err());
        assert!(
            decode_object(object(owner(4, None, None)), CHAIN, &"c".repeat(64), false).is_err()
        );
    }

    #[test]
    fn rejects_missing_requested_contents() {
        assert!(decode_object(object(owner(4, None, None)), CHAIN, &"a".repeat(64), true).is_err());
    }

    #[test]
    fn validates_explicit_https_and_expected_chain_id() {
        assert!(SuiGrpcNetwork::new("http://localhost:9000", CHAIN).is_err());
        assert!(SuiGrpcNetwork::new("https://node.example", " ").is_err());
    }
}
