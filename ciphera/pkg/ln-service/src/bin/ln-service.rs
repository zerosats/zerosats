use actix_cors::Cors;
use actix_web::{App, HttpServer, web};
use clap::Parser;
use ln_service::clients::{MempoolClient, PhoenixdClient, ReqwestCipheraClient};
use ln_service::config::Config;
use ln_service::db;
use ln_service::http::{AppState, configure_routes};
use ln_service::settlement::{SettlementContext, run_supervisor};
use ln_service::settlement::proof::LocalEscrowClaimProver;
use std::sync::Arc;
use std::time::Duration;

#[actix_web::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("ln_service=info,actix_web=info")),
        )
        .init();

    let config = Arc::new(Config::parse());

    // Resolve the default WCBTC note kind once at startup so request
    // handlers don't have to re-evaluate the Citrea-network → note-kind
    // mapping on every call. Fails fast on misconfiguration.
    let default_note_kind = config
        .resolved_default_note_kind()
        .map_err(|e| eyre::eyre!("note kind config: {e}"))?;
    tracing::info!(?default_note_kind, "resolved default note kind");

    let pool = db::connect(&config.db_path).await?;

    let chain_tip = Arc::new(MempoolClient::new(config.mempool_url.clone()));
    let ciphera = Arc::new(ReqwestCipheraClient::new(config.ciphera_url.clone()));
    let lightning = Arc::new(PhoenixdClient::new(
        &config.phoenixd_api_password,
        &config.phoenixd_url,
    )?);
    let prover = Arc::new(LocalEscrowClaimProver);

    let settlement_ctx = SettlementContext {
        db: pool.clone(),
        ciphera: ciphera.clone(),
        lightning,
        prover,
        service_evm_address: config.service_evm_address,
        service_secret_key: config.service_secret_key,
        tick: Duration::from_millis(config.worker_tick_ms),
    };
    tokio::spawn(run_supervisor(settlement_ctx));

    let bind = config.bind.clone();
    let app_state = AppState {
        config: config.clone(),
        db: pool,
        chain_tip,
        default_note_kind,
    };

    tracing::info!(%bind, "starting ln-service HTTP server");
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
