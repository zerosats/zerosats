use actix_web::{web, HttpResponse};
use prometheus::{Encoder, IntCounter, IntGauge, Registry, TextEncoder};
use std::sync::LazyLock;

use super::state::State;

static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::new);

static BLOCK_HEIGHT: LazyLock<IntGauge> = LazyLock::new(|| {
    let g = IntGauge::new("ciphera_block_height", "Current block height").unwrap();
    REGISTRY.register(Box::new(g.clone())).unwrap();
    g
});

static MEMPOOL_SIZE: LazyLock<IntGauge> = LazyLock::new(|| {
    let g =
        IntGauge::new("ciphera_mempool_size", "Pending transactions in mempool").unwrap();
    REGISTRY.register(Box::new(g.clone())).unwrap();
    g
});

static TRANSACTIONS_TOTAL: LazyLock<IntCounter> = LazyLock::new(|| {
    let c =
        IntCounter::new("ciphera_transactions_total", "Total transactions processed").unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

static MERKLE_TREE_SIZE: LazyLock<IntGauge> = LazyLock::new(|| {
    let g = IntGauge::new("ciphera_merkle_tree_elements", "Elements in merkle tree").unwrap();
    REGISTRY.register(Box::new(g.clone())).unwrap();
    g
});

/// Increment transaction counter. Called from txn.rs on successful submission.
pub fn inc_transactions() {
    TRANSACTIONS_TOTAL.inc();
}

pub async fn get_metrics(state: web::Data<State>) -> HttpResponse {
    let node = &state.node;

    BLOCK_HEIGHT.set(node.height().0 as i64);
    MEMPOOL_SIZE.set(node.mempool_len() as i64);
    MERKLE_TREE_SIZE.set(node.tree_len() as i64);

    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    let _ = encoder.encode(&REGISTRY.gather(), &mut buffer);
    let _ = encoder.encode(&prometheus::gather(), &mut buffer);

    HttpResponse::Ok()
        .content_type("text/plain; version=0.0.4")
        .body(buffer)
}
