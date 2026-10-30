use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use prost_types::FieldMask;
use serde::Deserialize;
use std::collections::{HashSet, VecDeque};
use std::time::Duration;
use sui_rpc::{
    Client,
    proto::sui::rpc::v2::{ListOwnedObjectsRequest, ListOwnedObjectsResponse, Object},
};

const COIN_PAGE_SIZE: u32 = 1_000;
const MAX_COIN_PAGES: usize = 100;
const MAX_COIN_OBJECTS: usize = 10_000;
const INVENTORY_DEADLINE: Duration = Duration::from_secs(30);
const COIN_OBJECT_READ_MASK: &[&str] = &[
    "object_id",
    "version",
    "digest",
    "owner",
    "object_type",
    "contents",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrpcCoinObject {
    pub object_id: String,
    pub version: u64,
    pub digest: String,
    pub balance: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrpcCoinInventory {
    pub selected_coins: Vec<GrpcCoinObject>,
    pub selected_balance: u64,
    pub total_balance: u64,
}

#[derive(Debug, Clone, Copy)]
struct InventoryLimits {
    page_size: u32,
    max_pages: usize,
    max_objects: usize,
    deadline: Duration,
}

impl Default for InventoryLimits {
    fn default() -> Self {
        Self {
            page_size: COIN_PAGE_SIZE,
            max_pages: MAX_COIN_PAGES,
            max_objects: MAX_COIN_OBJECTS,
            deadline: INVENTORY_DEADLINE,
        }
    }
}

#[async_trait]
trait CoinPageSource: Send {
    async fn list_owned_objects(
        &mut self,
        request: ListOwnedObjectsRequest,
    ) -> Result<ListOwnedObjectsResponse>;
}

struct SuiRpcCoinPageSource {
    client: Client,
}

impl SuiRpcCoinPageSource {
    fn new(endpoint: &str) -> Result<Self> {
        if !endpoint.starts_with("https://") {
            bail!("Sui gRPC endpoint must use HTTPS");
        }
        let client = Client::new(endpoint).context("Failed to construct Sui gRPC client")?;
        Ok(Self { client })
    }
}

#[async_trait]
impl CoinPageSource for SuiRpcCoinPageSource {
    async fn list_owned_objects(
        &mut self,
        request: ListOwnedObjectsRequest,
    ) -> Result<ListOwnedObjectsResponse> {
        self.client
            .state_client()
            .list_owned_objects(tonic::Request::new(request))
            .await
            .map(tonic::Response::into_inner)
            .context("Sui gRPC ListOwnedObjects request failed")
    }
}

/// Read a complete, bounded inventory of concrete Coin<T> objects through Sui gRPC.
/// This is deliberately separate from the legacy transaction builder during Stage 1.
pub async fn get_coin_objects_by_amount(
    endpoint: &str,
    owner: &str,
    coin_type: &str,
    amount: u64,
) -> Result<GrpcCoinInventory> {
    if amount == 0 {
        bail!("Requested coin amount must be greater than zero");
    }
    validate_owner(owner)?;
    validate_coin_type(coin_type)?;

    let mut source = SuiRpcCoinPageSource::new(endpoint)?;
    collect_coin_inventory(
        &mut source,
        owner,
        coin_type,
        amount,
        InventoryLimits::default(),
    )
    .await
}

async fn collect_coin_inventory<S: CoinPageSource>(
    source: &mut S,
    owner: &str,
    coin_type: &str,
    amount: u64,
    limits: InventoryLimits,
) -> Result<GrpcCoinInventory> {
    if amount == 0 {
        bail!("Requested coin amount must be greater than zero");
    }
    validate_owner(owner)?;
    validate_coin_type(coin_type)?;
    if limits.page_size == 0 || limits.max_pages == 0 || limits.max_objects == 0 {
        bail!("Sui gRPC inventory limits must all be greater than zero");
    }

    let expected_type = format!("0x2::coin::Coin<{coin_type}>");
    let deadline = tokio::time::Instant::now() + limits.deadline;
    let mut page_token: Option<Vec<u8>> = None;
    let mut seen_tokens: HashSet<Vec<u8>> = HashSet::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut coins = Vec::new();
    let mut total_balance = 0u64;

    for page_number in 1..=limits.max_pages {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            bail!("Sui gRPC coin inventory deadline exceeded; inventory is incomplete");
        }

        let request = ListOwnedObjectsRequest {
            owner: Some(owner.to_owned()),
            page_size: Some(limits.page_size),
            page_token: page_token.take(),
            read_mask: Some(FieldMask {
                paths: COIN_OBJECT_READ_MASK.iter().map(|path| (*path).to_owned()).collect(),
            }),
            object_type: Some(expected_type.clone()),
        };

        let response = tokio::time::timeout(remaining, source.list_owned_objects(request))
            .await
            .map_err(|_| anyhow!("Sui gRPC coin inventory deadline exceeded; inventory is incomplete"))?
            .with_context(|| format!("Sui gRPC coin inventory failed on page {page_number}"))?;

        if response.objects.len() > limits.page_size as usize {
            bail!("Sui gRPC page {page_number} exceeded its requested page size; inventory is incomplete");
        }

        let new_total = coins
            .len()
            .checked_add(response.objects.len())
            .ok_or_else(|| anyhow!("Sui gRPC coin object count overflow"))?;
        if new_total > limits.max_objects {
            bail!("Sui gRPC coin inventory exceeded the {}-object limit; inventory is incomplete", limits.max_objects);
        }

        for object in response.objects {
            let coin = decode_coin_object(object, owner, &expected_type)?;
            if !seen_ids.insert(coin.object_id.clone()) {
                bail!("Sui gRPC coin inventory returned a duplicate object ID; inventory is incomplete");
            }
            total_balance = total_balance
                .checked_add(coin.balance)
                .ok_or_else(|| anyhow!("Sui gRPC coin inventory balance overflow"))?;
            coins.push(coin);
        }

        match response.next_page_token {
            None => break,
            Some(token) => {
                if token.is_empty() {
                    bail!("Sui gRPC returned an empty continuation token; inventory is incomplete");
                }
                if !seen_tokens.insert(token.clone()) {
                    bail!("Sui gRPC repeated a continuation token; inventory is incomplete");
                }
                if page_number >= limits.max_pages {
                    bail!("Sui gRPC coin inventory exceeded the {}-page limit; inventory is incomplete", limits.max_pages);
                }
                page_token = Some(token);
            }
        }
    }

    coins.sort_by(|left, right| right.balance.cmp(&left.balance));
    let mut selected_coins = Vec::new();
    let mut selected_balance = 0u64;
    for coin in coins {
        selected_balance = selected_balance
            .checked_add(coin.balance)
            .ok_or_else(|| anyhow!("Sui gRPC selected coin balance overflow"))?;
        selected_coins.push(coin);
        if selected_balance >= amount {
            return Ok(GrpcCoinInventory {
                selected_coins,
                selected_balance,
                total_balance,
            });
        }
    }

    bail!("Insufficient balance: complete gRPC inventory contains {total_balance}, requested {amount}");
}

fn decode_coin_object(object: Object, owner: &str, expected_type: &str) -> Result<GrpcCoinObject> {
    let object_id = object.object_id.context("Sui gRPC coin object is missing object_id")?;
    validate_object_id(&object_id)?;
    let version = object.version.context("Sui gRPC coin object is missing version")?;
    if version == 0 {
        bail!("Sui gRPC coin object {object_id} has invalid version zero");
    }
    let digest = object.digest.context("Sui gRPC coin object {object_id} is missing digest")?;
    if digest.is_empty() {
        bail!("Sui gRPC coin object {object_id} has an empty digest");
    }
    let object_type = object.object_type.context("Sui gRPC coin object {object_id} is missing object_type")?;
    if object_type != expected_type {
        bail!("Sui gRPC coin object {object_id} has unexpected type {object_type}; expected {expected_type}");
    }
    let object_owner = object.owner.context("Sui gRPC coin object {object_id} is missing owner")?;
    if object_owner.kind != Some(1) || object_owner.address.as_deref() != Some(owner) {
        bail!("Sui gRPC coin object {object_id} is not address-owned by the requested owner");
    }

    let contents = object.contents.context("Sui gRPC coin object {object_id} is missing BCS contents")?;
    let bytes = contents.value.context("Sui gRPC coin object {object_id} has empty BCS contents")?;
    let (_, balance): ([u8; 32], u64) = bcs::from_bytes(&bytes)
        .with_context(|| format!("Sui gRPC coin object {object_id} has malformed Coin<T> BCS contents"))?;

    Ok(GrpcCoinObject {
        object_id,
        version,
        digest,
        balance,
    })
}

fn validate_owner(owner: &str) -> Result<()> {
    let value = owner.strip_prefix("0x").unwrap_or(owner);
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("Sui owner must be a 32-byte hexadecimal address");
    }
    Ok(())
}

fn validate_object_id(object_id: &str) -> Result<()> {
    let value = object_id.strip_prefix("0x").unwrap_or(object_id);
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("Sui gRPC returned an invalid 32-byte coin object ID");
    }
    Ok(())
}

fn validate_coin_type(coin_type: &str) -> Result<()> {
    if coin_type.is_empty() || !coin_type.contains("::") || coin_type.contains('<') || coin_type.contains('>') {
        bail!("Coin type must be a fully qualified, non-generic Move type");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CoinPageSource, GrpcCoinInventory, InventoryLimits, collect_coin_inventory,
    };
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use prost::bytes::Bytes;
    use std::{collections::VecDeque, time::Duration};
    use sui_rpc::proto::sui::rpc::v2::{
        Bcs, ListOwnedObjectsRequest, ListOwnedObjectsResponse, Object, Owner,
    };

    const OWNER: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const COIN_TYPE: &str = "0x2::sui::SUI";
    const EXPECTED_TYPE: &str = "0x2::coin::Coin<0x2::sui::SUI>";

    struct MockGrpcSource {
        pages: VecDeque<Result<ListOwnedObjectsResponse>>,
        delay: Duration,
    }

    #[async_trait]
    impl CoinPageSource for MockGrpcSource {
        async fn list_owned_objects(
            &mut self,
            request: ListOwnedObjectsRequest,
        ) -> Result<ListOwnedObjectsResponse> {
            assert_eq!(request.owner.as_deref(), Some(OWNER));
            assert_eq!(request.object_type.as_deref(), Some(EXPECTED_TYPE));
            assert_eq!(request.page_size, Some(10));
            assert!(request.read_mask.as_ref().unwrap().paths.contains(&"contents".to_owned()));
            tokio::time::sleep(self.delay).await;
            self.pages.pop_front().unwrap_or_else(|| Err(anyhow!("mock page queue exhausted")))
        }
    }

    fn object(id_byte: u8, balance: u64) -> Object {
        let contents = bcs::to_bytes(&([id_byte; 32], balance)).unwrap();
        Object {
            object_id: Some(format!("0x{}", hex_id(id_byte))),
            version: Some(1),
            digest: Some("mock-digest".to_owned()),
            owner: Some(Owner { kind: Some(1), address: Some(OWNER.to_owned()), version: None }),
            object_type: Some(EXPECTED_TYPE.to_owned()),
            contents: Some(Bcs { name: None, value: Some(Bytes::from(contents)) }),
            ..Default::default()
        }
    }

    fn hex_id(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn page(objects: Vec<Object>, token: Option<&[u8]>) -> ListOwnedObjectsResponse {
        ListOwnedObjectsResponse {
            objects,
            next_page_token: token.map(|value| value.to_vec()),
        }
    }

    fn limits() -> InventoryLimits {
        InventoryLimits {
            page_size: 10,
            max_pages: 4,
            max_objects: 20,
            deadline: Duration::from_secs(1),
        }
    }

    fn source(pages: Vec<Result<ListOwnedObjectsResponse>>) -> MockGrpcSource {
        MockGrpcSource { pages: pages.into(), delay: Duration::ZERO }
    }

    #[tokio::test]
    async fn reads_multiple_pages_and_selects_largest_coins() {
        let mut mock = source(vec![
            Ok(page(vec![object(1, 4)], Some(b"next"))),
            Ok(page(vec![object(2, 9)], None)),
        ]);
        let result = collect_coin_inventory(&mut mock, OWNER, COIN_TYPE, 8, limits()).await.unwrap();
        assert_eq!(result.selected_balance, 9);
        assert_eq!(result.total_balance, 13);
        assert_eq!(result.selected_coins.len(), 1);
    }

    #[tokio::test]
    async fn follows_empty_page_with_continuation_token() {
        let mut mock = source(vec![
            Ok(page(vec![], Some(b"next"))),
            Ok(page(vec![object(3, 12)], None)),
        ]);
        let result = collect_coin_inventory(&mut mock, OWNER, COIN_TYPE, 10, limits()).await.unwrap();
        assert_eq!(result.selected_balance, 12);
    }

    #[tokio::test]
    async fn rpc_error_fails_closed_instead_of_returning_partial_inventory() {
        let mut mock = source(vec![
            Ok(page(vec![object(4, 100)], Some(b"next"))),
            Err(anyhow!("mock transport failure")),
        ]);
        assert!(collect_coin_inventory(&mut mock, OWNER, COIN_TYPE, 1, limits()).await.is_err());
    }

    #[tokio::test]
    async fn deadline_fails_closed() {
        let mut mock = source(vec![Ok(page(vec![object(5, 100)], None))]);
        mock.delay = Duration::from_millis(30);
        let mut short = limits();
        short.deadline = Duration::from_millis(1);
        assert!(collect_coin_inventory(&mut mock, OWNER, COIN_TYPE, 1, short).await.is_err());
    }

    #[tokio::test]
    async fn malformed_bcs_fails_closed() {
        let mut malformed = object(6, 1);
        malformed.contents = Some(Bcs { name: None, value: Some(Bytes::from_static(b"bad")) });
        let mut mock = source(vec![Ok(page(vec![malformed], None))]);
        assert!(collect_coin_inventory(&mut mock, OWNER, COIN_TYPE, 1, limits()).await.is_err());
    }

    #[tokio::test]
    async fn missing_bcs_fails_closed() {
        let mut missing = object(7, 1);
        missing.contents = None;
        let mut mock = source(vec![Ok(page(vec![missing], None))]);
        assert!(collect_coin_inventory(&mut mock, OWNER, COIN_TYPE, 1, limits()).await.is_err());
    }

    #[tokio::test]
    async fn wrong_coin_type_fails_closed() {
        let mut wrong = object(8, 1);
        wrong.object_type = Some("0x2::coin::Coin<0x2::other::OTHER>".to_owned());
        let mut mock = source(vec![Ok(page(vec![wrong], None))]);
        assert!(collect_coin_inventory(&mut mock, OWNER, COIN_TYPE, 1, limits()).await.is_err());
    }

    #[tokio::test]
    async fn wrong_owner_fails_closed() {
        let mut wrong = object(9, 1);
        wrong.owner.as_mut().unwrap().address = Some("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned());
        let mut mock = source(vec![Ok(page(vec![wrong], None))]);
        assert!(collect_coin_inventory(&mut mock, OWNER, COIN_TYPE, 1, limits()).await.is_err());
    }

    #[tokio::test]
    async fn duplicate_object_id_fails_closed() {
        let mut mock = source(vec![
            Ok(page(vec![object(10, 2)], Some(b"next"))),
            Ok(page(vec![object(10, 3)], None)),
        ]);
        assert!(collect_coin_inventory(&mut mock, OWNER, COIN_TYPE, 1, limits()).await.is_err());
    }

    #[tokio::test]
    async fn repeated_page_token_fails_closed() {
        let mut mock = source(vec![
            Ok(page(vec![], Some(b"same"))),
            Ok(page(vec![], Some(b"same"))),
        ]);
        assert!(collect_coin_inventory(&mut mock, OWNER, COIN_TYPE, 1, limits()).await.is_err());
    }

    #[tokio::test]
    async fn page_cap_fails_closed() {
        let mut mock = source(vec![Ok(page(vec![], Some(b"next")))]);
        let mut bounded = limits();
        bounded.max_pages = 1;
        assert!(collect_coin_inventory(&mut mock, OWNER, COIN_TYPE, 1, bounded).await.is_err());
    }

    #[tokio::test]
    async fn object_cap_fails_closed() {
        let mut mock = source(vec![Ok(page(vec![object(11, 2), object(12, 3)], None))]);
        let mut bounded = limits();
        bounded.max_objects = 1;
        assert!(collect_coin_inventory(&mut mock, OWNER, COIN_TYPE, 1, bounded).await.is_err());
    }

    #[tokio::test]
    async fn insufficient_balance_reports_complete_inventory_as_error() {
        let mut mock = source(vec![Ok(page(vec![object(13, 2)], None))]);
        assert!(collect_coin_inventory(&mut mock, OWNER, COIN_TYPE, 3, limits()).await.is_err());
    }

    #[tokio::test]
    async fn empty_token_fails_closed() {
        let mut mock = source(vec![Ok(page(vec![], Some(b"")))]);
        assert!(collect_coin_inventory(&mut mock, OWNER, COIN_TYPE, 1, limits()).await.is_err());
    }

    #[tokio::test]
    async fn inventory_result_type_is_stable() {
        let _ = std::mem::size_of::<GrpcCoinInventory>();
    }
}
