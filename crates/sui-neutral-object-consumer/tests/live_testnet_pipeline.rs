//! Live, read-only Sui Testnet gRPC → neutral producer → consumer pipeline.
//!
//! This test performs only GetServiceInfo and GetObject RPCs. It does not
//! construct transaction arguments, sign, submit, or mutate chain state.

use sui_grpc_inventory::{SuiGrpcNetwork, SuiGrpcObjectReader, neutral::to_neutral_record};
use sui_neutral_object_boundary::NeutralObjectRecord;
use sui_neutral_object_consumer::parse_neutral_json;

const TESTNET_ENDPOINT: &str = "https://fullnode.testnet.sui.io:443";
const EXPECTED_TESTNET_CHAIN_ID: &str = "69WiPg3DAQiwdxfncX6wYQ2siKwAe6L9BZthQea3JNMD";
const SYSTEM_STATE_OBJECT_ID: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000005";

#[tokio::test]
async fn live_testnet_object_round_trips_through_neutral_pipeline() {
    let network = SuiGrpcNetwork::new(TESTNET_ENDPOINT, EXPECTED_TESTNET_CHAIN_ID)
        .expect("pinned Testnet configuration must be valid");
    let mut reader = SuiGrpcObjectReader::connect(network)
        .await
        .expect("endpoint must prove the pinned Testnet chain identity");

    assert_eq!(reader.chain_id(), EXPECTED_TESTNET_CHAIN_ID);

    let source = reader
        .get_object_with_contents(SYSTEM_STATE_OBJECT_ID, true)
        .await
        .expect("read-only GetObject for Sui system state must succeed");

    assert_eq!(source.chain_id, EXPECTED_TESTNET_CHAIN_ID);
    assert_eq!(
        source.object_id.to_ascii_lowercase(),
        SYSTEM_STATE_OBJECT_ID
    );
    assert!(source.version > 0, "live object version must be nonzero");
    assert!(
        !source.digest.is_empty(),
        "live object digest must be present"
    );
    assert!(
        !source.object_type.is_empty(),
        "live object type must be present"
    );
    assert!(
        source
            .contents_bcs
            .as_ref()
            .is_some_and(|bytes| !bytes.is_empty()),
        "requested live BCS contents must be present"
    );

    // Producer converts the exact live gRPC record to the shared neutral schema.
    let expected_neutral = to_neutral_record(&source)
        .expect("live gRPC record must satisfy producer-side schema validation");
    let producer_json = sui_grpc_inventory::neutral::to_neutral_json(&source)
        .expect("producer must serialize the live record");

    // Consumer parses producer output with no manual field rewriting.
    let consumed = parse_neutral_json(&producer_json)
        .expect("consumer must accept the producer's live schema-v1 JSON");

    assert_eq!(consumed, expected_neutral);
    assert_eq!(consumed.chain_id, EXPECTED_TESTNET_CHAIN_ID);
    assert_eq!(consumed.object_id, SYSTEM_STATE_OBJECT_ID);
    assert_eq!(consumed.version, source.version);
    assert_eq!(consumed.digest, source.digest);
    assert_eq!(consumed.object_type, source.object_type);
    assert_eq!(
        consumed.contents_bcs_base64,
        expected_neutral.contents_bcs_base64
    );

    // Independently deserialize the serialized JSON to ensure the public schema
    // representation is still directly consumable as the shared contract type.
    let decoded: NeutralObjectRecord =
        serde_json::from_str(&producer_json).expect("neutral JSON must deserialize");
    decoded.validate().expect("neutral record must validate");

    println!(
        "LIVE_SUI_NEUTRAL_PIPELINE_OK chain_id={} object_id={} version={} type={} bcs_bytes={}",
        consumed.chain_id,
        consumed.object_id,
        consumed.version,
        consumed.object_type,
        source.contents_bcs.as_ref().map_or(0, Vec::len)
    );
}
