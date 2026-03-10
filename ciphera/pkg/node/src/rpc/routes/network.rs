use super::State;
use actix_web::web;
use node_interface::NetworkResponse;
use rpc::error::HttpResult;

/// GET /height - returns data about the rollup (e.g. root hash, version, etc)

#[tracing::instrument(err, skip(state))]
pub async fn get_network_info(state: web::Data<State>) -> HttpResult<web::Json<NetworkResponse>> {
    Ok(web::Json(NetworkResponse {
        rollup_contract: state.node.rollup_contract(),
        chain_id: state.node.chain_id(),
    }))
}
