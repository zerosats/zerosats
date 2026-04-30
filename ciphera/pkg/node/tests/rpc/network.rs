use testutil::eth::EthNode;

use crate::rpc::ServerConfig;

use super::Server;

#[tokio::test(flavor = "multi_thread")]
async fn network_info() {
    let eth_node = EthNode::default().run_and_deploy().await;
    let server =
        Server::setup_and_wait(ServerConfig::single_node(false, &eth_node), eth_node).await;
    let resp = server.network().await.unwrap();
    let address_str = format!("0x{:x}", server.rollup_contract_addr);
    assert_eq!(resp.rollup_contract, address_str);
    assert_eq!(resp.chain_id, 5655);
    assert!(resp.escrow_manager.starts_with("0x"));
    assert_eq!(resp.escrow_manager.len(), 42);
    assert!(!resp.node_version.is_empty());
    assert_eq!(resp.circuits_nargo_version, constants::CIRCUITS_NARGO_VERSION);
    assert_eq!(resp.circuits_bb_version, constants::CIRCUITS_BB_VERSION);
}
