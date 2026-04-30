use super::State;
use actix_web::web;
use constants::{CIRCUITS_BB_VERSION, CIRCUITS_NARGO_VERSION};
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
        circuits_nargo_version: CIRCUITS_NARGO_VERSION.to_string(),
        circuits_bb_version: CIRCUITS_BB_VERSION.to_string(),
    }))
}
