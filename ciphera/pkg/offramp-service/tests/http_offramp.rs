//! Smoke test for `POST /v0/offramp`, `GET /v0/offramp/{id}`, and
//! `POST /v0/offramp/{id}/cancel`. We mint a fresh bolt11 invoice on the
//! fly (signed with a throwaway key) so the bolt11 isn't expired, and we
//! stub the chain-tip client to return a fixed block hash.

use actix_web::{App, test, web};
use async_trait::async_trait;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use element::Element;
use lightning_invoice::{Currency, InvoiceBuilder};
use lightning_types::payment::PaymentSecret;
use offramp_service::clients::ChainTipClient;
use offramp_service::config::Config;
use offramp_service::db;
use offramp_service::http::{AppState, configure_routes};
use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;

struct FixedTip(pub [u8; 32]);

#[async_trait]
impl ChainTipClient for FixedTip {
    async fn tip_hash(&self) -> eyre::Result<[u8; 32]> {
        Ok(self.0)
    }
}

fn mint_bolt11(amount_msat: u64) -> String {
    let private_key = SecretKey::from_slice(&[0x42u8; 32]).unwrap();
    let payment_hash = sha256::Hash::from_slice(&[0x11u8; 32][..]).unwrap();
    let payment_secret = PaymentSecret([0x22u8; 32]);
    InvoiceBuilder::new(Currency::Bitcoin)
        .description("offramp test".into())
        .payment_hash(payment_hash)
        .payment_secret(payment_secret)
        .current_timestamp()
        .min_final_cltv_expiry_delta(144)
        .amount_milli_satoshis(amount_msat)
        .build_signed(|h| Secp256k1::new().sign_ecdsa_recoverable(h, &private_key))
        .unwrap()
        .to_string()
}

/// Test fixture default note kind. The actual value doesn't matter — handlers
/// only compare it against whatever the `AppState` exposes.
fn test_note_kind() -> Element {
    Element::new(0xb7c)
}

fn make_config(db_path: &std::path::Path) -> Config {
    Config {
        bind: "127.0.0.1:0".into(),
        rollup_url: "http://nope".into(),
        phoenixd_url: "http://nope".into(),
        phoenixd_api_password: "x".into(),
        mempool_url: "http://nope".into(),
        db_path: db_path.to_string_lossy().into(),
        citrea_network: None,
        ciphera_btc_note_kind: Some(test_note_kind()),
        service_evm_address: Element::new(0x1234),
        timelock_n_blocks: 2,
        quote_ttl_seconds: 600,
        max_amount_sat: 1_000_000,
        worker_tick_ms: 1_000,
    }
}

#[actix_web::test]
async fn post_offramp_persists_quote() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("offramp.sqlite");
    let pool = db::connect(db_path.to_str().unwrap()).await.unwrap();
    let state = AppState {
        config: Arc::new(make_config(&db_path)),
        db: pool,
        chain_tip: Arc::new(FixedTip([0x7; 32])),
        default_note_kind: test_note_kind(),
    };
    let app = test::init_service(
        App::new().service(web::scope("").configure(configure_routes(state))),
    )
    .await;

    let bolt11 = mint_bolt11(100_000);

    let req = test::TestRequest::post()
        .uri("/v0/offramp")
        .set_json(&serde_json::json!({ "bolt11": bolt11, "address": "1" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {:?}", resp.status());
    let body: Value = test::read_body_json(resp).await;
    assert!(body["quote_id"].is_string());
    assert_eq!(body["timelock"]["n_blocks"], 2);
    assert_eq!(body["note"]["utxo_kind"], "2");

    let qid = body["quote_id"].as_str().unwrap();
    let req = test::TestRequest::get()
        .uri(&format!("/v0/offramp/{qid}"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "Pending");
}

#[actix_web::test]
async fn cancel_pending_quote_transitions_to_cancelled() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("offramp.sqlite");
    let pool = db::connect(db_path.to_str().unwrap()).await.unwrap();
    let state = AppState {
        config: Arc::new(make_config(&db_path)),
        db: pool,
        chain_tip: Arc::new(FixedTip([0x7; 32])),
        default_note_kind: test_note_kind(),
    };
    let app = test::init_service(
        App::new().service(web::scope("").configure(configure_routes(state))),
    )
    .await;

    let bolt11 = mint_bolt11(100_000);
    let req = test::TestRequest::post()
        .uri("/v0/offramp")
        .set_json(&serde_json::json!({ "bolt11": bolt11, "address": "1" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    let body: Value = test::read_body_json(resp).await;
    let qid = body["quote_id"].as_str().unwrap().to_string();

    let req = test::TestRequest::post()
        .uri(&format!("/v0/offramp/{qid}/cancel"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {:?}", resp.status());
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "Cancelled");
}

#[actix_web::test]
async fn rejects_invalid_bolt11() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("offramp.sqlite");
    let pool = db::connect(db_path.to_str().unwrap()).await.unwrap();
    let state = AppState {
        config: Arc::new(make_config(&db_path)),
        db: pool,
        chain_tip: Arc::new(FixedTip([0x7; 32])),
        default_note_kind: test_note_kind(),
    };
    let app = test::init_service(
        App::new().service(web::scope("").configure(configure_routes(state))),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v0/offramp")
        .set_json(&serde_json::json!({ "bolt11": "not-a-real-invoice", "address": "1" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 400);
}
