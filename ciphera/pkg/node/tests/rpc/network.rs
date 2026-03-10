use constants::MERKLE_TREE_DEPTH;
use testutil::eth::EthNode;

use crate::rpc::ServerConfig;

use super::Server;

#[tokio::test(flavor = "multi_thread")]
async fn network_info() {
    let eth_node = EthNode::default().run_and_deploy().await;
    let server = Server::setup_and_wait(ServerConfig::single_node(false), eth_node).await;
    let resp = server.network().await.unwrap();
    assert_eq!(resp.rollup_contract, "0xx");
    assert_eq!(resp.chain_id, 5115);
}
