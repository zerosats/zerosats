use actix_cors::Cors;
use actix_web::{App, HttpServer, web};
use clap::Parser;
use offramp_service::clients::{MempoolClient, PhoenixdClient, ReqwestRollupClient};
use offramp_service::config::Config;
use offramp_service::db;
use offramp_service::http::{AppState, configure_routes};
use offramp_service::settlement::{SettlementContext, run_supervisor};
use offramp_service::settlement::proof::LocalBurnProver;
use std::sync::Arc;
use std::time::Duration;

#[actix_web::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("offramp_service=info,actix_web=info")),
        )
        .init();

    let config = Arc::new(Config::parse());

    let pool = db::connect(&config.db_path).await?;

    let chain_tip = Arc::new(MempoolClient::new(config.mempool_url.clone()));
    let rollup = Arc::new(ReqwestRollupClient::new(config.rollup_url.clone()));
    let lightning = Arc::new(PhoenixdClient::new(
        &config.phoenixd_api_password,
        &config.phoenixd_url,
    )?);
    let prover = Arc::new(LocalBurnProver::default());

    let settlement_ctx = SettlementContext {
        db: pool.clone(),
        rollup: rollup.clone(),
        lightning,
        prover,
        service_evm_address: config.service_evm_address,
        tick: Duration::from_millis(config.worker_tick_ms),
    };
    tokio::spawn(run_supervisor(settlement_ctx));

    let bind = config.bind.clone();
    let app_state = AppState {
        config: config.clone(),
        db: pool,
        chain_tip,
    };

    tracing::info!(%bind, "starting offramp-service HTTP server");
    HttpServer::new(move || {
        App::new()
            .wrap(Cors::permissive())
            .service(web::scope("").configure(configure_routes(app_state.clone())))
    })
    .bind(&bind)?
    .run()
    .await?;
    Ok(())
}
