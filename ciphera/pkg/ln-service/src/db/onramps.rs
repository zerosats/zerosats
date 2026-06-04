use crate::db::DbPool;
use crate::domain::{Onramp, OnrampStatus};
use chrono::{DateTime, TimeZone, Utc};
use element::Element;
use sqlx::Row;
use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("onramp not found")]
    NotFound,
    #[error("payment hash already in use by another onramp")]
    DuplicatePaymentHash,
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid row: {0}")]
    Row(String),
}

pub async fn insert(pool: &DbPool, o: &Onramp) -> Result<(), Error> {
    let res = sqlx::query(
        r#"
        INSERT INTO onramps (
            payment_hash,
            bolt11,
            amount,
            note_kind,
            preimage,
            note_commitment,
            txn_hash,
            refund_txn_hash,
            status,
            last_error,
            expires_at,
            created_at,
            updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13);
        "#,
    )
    .bind(o.payment_hash.to_vec())
    .bind(&o.bolt11)
    .bind(element_bytes(&o.amount))
    .bind(element_bytes(&o.note_kind))
    .bind(o.preimage.to_vec())
    .bind(o.note_commitment.as_ref().map(element_bytes))
    .bind(o.txn_hash.as_ref().map(element_bytes))
    .bind(o.refund_txn_hash.as_ref().map(element_bytes))
    .bind(o.status.as_str())
    .bind(&o.last_error)
    .bind(o.expires_at.timestamp())
    .bind(o.created_at.timestamp())
    .bind(o.updated_at.timestamp())
    .execute(pool)
    .await;

    match res {
        Ok(_) => Ok(()),
        Err(sqlx::Error::Database(db)) if db.message().contains("payment_hash") => {
            Err(Error::DuplicatePaymentHash)
        }
        Err(e) => Err(Error::Sqlx(e)),
    }
}

pub async fn get(pool: &DbPool, payment_hash: [u8; 32]) -> Result<Onramp, Error> {
    let row = sqlx::query(r#"SELECT * FROM onramps WHERE payment_hash = ?1"#)
        .bind(payment_hash.to_vec())
        .fetch_optional(pool)
        .await?;
    row_to_onramp(row.ok_or(Error::NotFound)?)
}

/// Onramps the worker should still tick (non-terminal), oldest first.
pub async fn list_non_terminal(pool: &DbPool) -> Result<Vec<Onramp>, Error> {
    let rows = sqlx::query(
        r#"
        SELECT * FROM onramps
        WHERE status NOT IN ('Settled', 'Refunded', 'Failed')
        ORDER BY created_at ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_onramp).collect()
}

pub async fn update_status(
    pool: &DbPool,
    payment_hash: [u8; 32],
    status: OnrampStatus,
) -> Result<(), Error> {
    sqlx::query(r#"UPDATE onramps SET status = ?1, updated_at = ?2 WHERE payment_hash = ?3"#)
        .bind(status.as_str())
        .bind(Utc::now().timestamp())
        .bind(payment_hash.to_vec())
        .execute(pool)
        .await?;
    Ok(())
}

/// Stamp the funding-Send commitment + tx hash and transition to
/// `Committed`.
pub async fn record_committed(
    pool: &DbPool,
    payment_hash: [u8; 32],
    note_commitment: Element,
    txn_hash: Element,
) -> Result<(), Error> {
    sqlx::query(
        r#"
        UPDATE onramps
        SET status = 'Committed',
            note_commitment = ?1,
            txn_hash = ?2,
            updated_at = ?3
        WHERE payment_hash = ?4
        "#,
    )
    .bind(element_bytes(&note_commitment))
    .bind(element_bytes(&txn_hash))
    .bind(Utc::now().timestamp())
    .bind(payment_hash.to_vec())
    .execute(pool)
    .await?;
    Ok(())
}

/// Stamp the refund-Send tx hash and transition to `Refunding`.
pub async fn record_refunding(
    pool: &DbPool,
    payment_hash: [u8; 32],
    refund_txn_hash: Element,
) -> Result<(), Error> {
    sqlx::query(
        r#"
        UPDATE onramps
        SET status = 'Refunding',
            refund_txn_hash = ?1,
            updated_at = ?2
        WHERE payment_hash = ?3
        "#,
    )
    .bind(element_bytes(&refund_txn_hash))
    .bind(Utc::now().timestamp())
    .bind(payment_hash.to_vec())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_error(
    pool: &DbPool,
    payment_hash: [u8; 32],
    message: &str,
) -> Result<(), Error> {
    sqlx::query(r#"UPDATE onramps SET last_error = ?1, updated_at = ?2 WHERE payment_hash = ?3"#)
        .bind(message)
        .bind(Utc::now().timestamp())
        .bind(payment_hash.to_vec())
        .execute(pool)
        .await?;
    Ok(())
}

fn element_bytes(e: &Element) -> Vec<u8> {
    e.to_be_bytes().to_vec()
}

fn element_from_bytes(b: &[u8]) -> Result<Element, Error> {
    if b.len() != 32 {
        return Err(Error::Row(format!("expected 32-byte Element, got {}", b.len())));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(b);
    Ok(Element::from_be_bytes(arr))
}

fn fixed32(b: &[u8]) -> Result<[u8; 32], Error> {
    if b.len() != 32 {
        return Err(Error::Row(format!("expected 32 bytes, got {}", b.len())));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(b);
    Ok(arr)
}

fn ts(secs: i64) -> Result<DateTime<Utc>, Error> {
    Utc.timestamp_opt(secs, 0)
        .single()
        .ok_or_else(|| Error::Row(format!("invalid timestamp {secs}")))
}

fn row_to_onramp(row: sqlx::sqlite::SqliteRow) -> Result<Onramp, Error> {
    let payment_hash: Vec<u8> = row.try_get("payment_hash")?;
    let bolt11: String = row.try_get("bolt11")?;
    let amount: Vec<u8> = row.try_get("amount")?;
    let note_kind: Vec<u8> = row.try_get("note_kind")?;
    let preimage: Vec<u8> = row.try_get("preimage")?;
    let note_commitment: Option<Vec<u8>> = row.try_get("note_commitment")?;
    let txn_hash: Option<Vec<u8>> = row.try_get("txn_hash")?;
    let refund_txn_hash: Option<Vec<u8>> = row.try_get("refund_txn_hash")?;
    let status_str: String = row.try_get("status")?;
    let last_error: Option<String> = row.try_get("last_error")?;
    let expires_at: i64 = row.try_get("expires_at")?;
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;

    Ok(Onramp {
        payment_hash: fixed32(&payment_hash)?,
        bolt11,
        amount: element_from_bytes(&amount)?,
        note_kind: element_from_bytes(&note_kind)?,
        preimage: fixed32(&preimage)?,
        note_commitment: note_commitment.map(|b| element_from_bytes(&b)).transpose()?,
        txn_hash: txn_hash.map(|b| element_from_bytes(&b)).transpose()?,
        refund_txn_hash: refund_txn_hash.map(|b| element_from_bytes(&b)).transpose()?,
        status: OnrampStatus::from_str(&status_str).map_err(Error::Row)?,
        last_error,
        expires_at: ts(expires_at)?,
        created_at: ts(created_at)?,
        updated_at: ts(updated_at)?,
    })
}
