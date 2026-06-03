//! Smoke test for `GET /v0/onramp` and `GET /v0/onramp/{payment_hash}`.
//! Lightning is stubbed to return a canned invoice, so no phoenixd is
//! needed; we only exercise the HTTP surface + persistence.

use actix_web::{App, test, web};
use async_trait::async_trait;
use element::Element;
use ln_service::clients::{
    ChainTipClient, CreatedInvoice, LightningClient, LightningPaymentStatus, PayResult,
};
use ln_service::config::Config;
use ln_service::db;
use ln_service::domain::ServiceNote;
use ln_service::http::{AppState, configure_routes};
use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;

/// Seed an unspent service note of the default kind so the onramp
/// liquidity precheck passes.
async fn seed_service_note(state: &AppState, value: u64) {
    let now = chrono::Utc::now();
    let note = ServiceNote {
        commitment: Element::new(0xdeadbeef),
        note_secret: Element::new(0x1111),
        note_kind: test_note_kind(),
        value,
        spent: false,
        source_payment_hash: None,
        created_at: now,
        updated_at: now,
    };
    db::service_notes::insert(&state.db, &note).await.unwrap();
}

const CANNED_PAYMENT_HASH: [u8; 32] = [0xab; 32];
const CANNED_BOLT11: &str = "lnbcrt10u1pcannedinvoiceplaceholder";

struct FixedTip;

#[async_trait]
impl ChainTipClient for FixedTip {
    async fn tip_hash(&self) -> eyre::Result<[u8; 32]> {
        Ok([0x7; 32])
    }
}

/// Returns a deterministic invoice from `create_invoice`; the other
/// methods are unused by the onramp HTTP endpoints.
struct CannedLightning;

#[async_trait]
impl LightningClient for CannedLightning {
    async fn pay_invoice(&self, _bolt11: &str) -> eyre::Result<PayResult> {
        eyre::bail!("unused")
    }
    async fn payment_status(&self, _hash: &str) -> eyre::Result<LightningPaymentStatus> {
        eyre::bail!("unused")
    }
    async fn create_invoice(&self, _amount_sat: u64, _desc: &str) -> eyre::Result<CreatedInvoice> {
        Ok(CreatedInvoice {
            payment_hash: CANNED_PAYMENT_HASH,
            bolt11: CANNED_BOLT11.to_string(),
        })
    }
    async fn incoming_preimage(&self, _hash: &str) -> eyre::Result<Option<[u8; 32]>> {
        Ok(None)
    }
}

fn test_note_kind() -> Element {
    Element::new(0xb7c)
}

fn make_config(db_path: &std::path::Path) -> Config {
    Config {
        bind: "127.0.0.1:0".into(),
        ciphera_url: "http://nope".into(),
        phoenixd_url: "http://nope".into(),
        phoenixd_api_password: "x".into(),
        mempool_url: "http://nope".into(),
        db_path: db_path.to_string_lossy().into(),
        chain_id: None,
        citrea_network: None,
        ciphera_btc_note_kind: Some(test_note_kind()),
        service_evm_address: Element::new(0x1234),
        service_secret_key: Element::new(0x5678),
        timelock_n_blocks: 2,
        quote_ttl_seconds: 600,
        max_amount_sat: 1_000_000,
        worker_tick_ms: 1_000,
    }
}

async fn make_app_state(db_path: &std::path::Path) -> AppState {
    let pool = db::connect(db_path.to_str().unwrap()).await.unwrap();
    AppState {
        config: Arc::new(make_config(db_path)),
        db: pool,
        chain_tip: Arc::new(FixedTip),
        lightning: Arc::new(CannedLightning),
        default_note_kind: test_note_kind(),
    }
}

#[actix_web::test]
async fn get_onramp_issues_invoice_and_persists() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("onramp.sqlite");
    let state = make_app_state(&db_path).await;
    // 1000 sat = 1e13 wei; seed a note that comfortably covers it.
    seed_service_note(&state, 1_000_000_000_000_000).await;
    let app = test::init_service(
        App::new().service(web::scope("").configure(configure_routes(state))),
    )
    .await;

    // Create the onramp.
    let req = test::TestRequest::get()
        .uri("/v0/onramp?amount_sat=1000")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {:?}", resp.status());
    let body: Value = test::read_body_json(resp).await;

    let expected_hash = hex::encode(CANNED_PAYMENT_HASH);
    assert_eq!(body["payment_hash"], expected_hash);
    assert_eq!(body["bolt11"], CANNED_BOLT11);
    assert_eq!(body["amount_sat"], 1000);
    assert_eq!(body["status"], "InvoicePending");

    // Fetch status by payment hash.
    let req = test::TestRequest::get()
        .uri(&format!("/v0/onramp/{expected_hash}"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "InvoicePending");
    assert_eq!(body["bolt11"], CANNED_BOLT11);
    // Unpaid: no preimage or note yet.
    assert!(body["preimage"].is_null());
    assert!(body["note_commitment"].is_null());
}

#[actix_web::test]
async fn get_onramp_rejects_when_no_liquidity() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("onramp.sqlite");
    let state = make_app_state(&db_path).await; // no service notes seeded
    let app = test::init_service(
        App::new().service(web::scope("").configure(configure_routes(state))),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/v0/onramp?amount_sat=1000")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::SERVICE_UNAVAILABLE
    );
}

#[actix_web::test]
async fn get_onramp_rejects_zero_amount() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("onramp.sqlite");
    let state = make_app_state(&db_path).await;
    let app = test::init_service(
        App::new().service(web::scope("").configure(configure_routes(state))),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/v0/onramp?amount_sat=0")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::BAD_REQUEST);
}

#[actix_web::test]
async fn get_onramp_rejects_over_cap() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("onramp.sqlite");
    let state = make_app_state(&db_path).await;
    let app = test::init_service(
        App::new().service(web::scope("").configure(configure_routes(state))),
    )
    .await;

    // max_amount_sat is 1_000_000 in the test config.
    let req = test::TestRequest::get()
        .uri("/v0/onramp?amount_sat=2000000")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::BAD_REQUEST);
}

#[actix_web::test]
async fn get_onramp_unknown_payment_hash_is_404() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("onramp.sqlite");
    let state = make_app_state(&db_path).await;
    let app = test::init_service(
        App::new().service(web::scope("").configure(configure_routes(state))),
    )
    .await;

    let unknown = hex::encode([0x01u8; 32]);
    let req = test::TestRequest::get()
        .uri(&format!("/v0/onramp/{unknown}"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::NOT_FOUND);
}
