pub mod error;
pub mod offramp;

use crate::clients::ChainTipClient;
use crate::config::Config;
use crate::db::DbPool;
use actix_web::web;
use std::sync::Arc;

/// Shared state injected into every actix handler.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: DbPool,
    pub chain_tip: Arc<dyn ChainTipClient>,
}

pub fn configure_routes(state: AppState) -> impl FnOnce(&mut web::ServiceConfig) {
    move |cfg: &mut web::ServiceConfig| {
        cfg.app_data(web::Data::new(state))
            .service(
                web::scope("/v0")
                    .service(
                        web::resource("/offramp").route(web::post().to(offramp::create_offramp)),
                    )
                    .service(
                        web::resource("/offramp/{quote_id}")
                            .route(web::get().to(offramp::get_offramp)),
                    )
                    .service(
                        web::resource("/offramp/{quote_id}/cancel")
                            .route(web::post().to(offramp::cancel_offramp)),
                    ),
            );
    }
}
