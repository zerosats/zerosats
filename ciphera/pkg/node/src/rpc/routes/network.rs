use super::State;
use actix_web::web;
use node_interface::NetworkResponse;
use rpc::error::HttpResult;

/// GET /network - returns data about the Ciphera network: main contract, chain id, nodes etc.

#[tracing::instrument(err, skip(state))]
pub async fn get_network_info(state: web::Data<State>) -> HttpResult<web::Json<NetworkResponse>> {
    let escrow_manager = state.node.escrow_manager().await?;
    Ok(web::Json(NetworkResponse {
        rollup_contract: state.node.rollup_contract(),
        chain_id: state.node.chain_id(),
        escrow_manager,
        node_version: env!("CARGO_PKG_VERSION").to_string(),
    }))
}
