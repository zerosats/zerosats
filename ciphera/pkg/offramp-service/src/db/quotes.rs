use crate::db::DbPool;
use crate::domain::{Quote, QuoteStatus};
use chrono::{DateTime, TimeZone, Utc};
use element::Element;
use sqlx::Row;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("quote not found")]
    NotFound,
    #[error("payment hash already in use by another quote")]
    DuplicatePaymentHash,
    #[error("note commitment already in use by another quote")]
    DuplicateNoteCommitment,
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid row: {0}")]
    Row(String),
}

pub async fn insert(pool: &DbPool, q: &Quote) -> Result<(), Error> {
    let res = sqlx::query(
        r#"
        INSERT INTO quotes (
          quote_id, status, bolt11, payment_hash, user_address, note_kind, amount,
          zero_block, n_blocks, note_commitment,
          preimage, lightning_payment_id, burn_txn_hash, last_error,
          expires_at, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17);
        "#,
    )
    .bind(q.quote_id.to_string())
    .bind(q.status.as_str())
    .bind(&q.bolt11)
    .bind(&q.payment_hash[..])
    .bind(element_bytes(&q.user_address))
    .bind(element_bytes(&q.note_kind))
    .bind(element_bytes(&q.amount))
    .bind(&q.zero_block[..])
    .bind(element_bytes(&q.n_blocks))
    .bind(element_bytes(&q.note_commitment))
    .bind(q.preimage.as_ref().map(|p| p.to_vec()))
    .bind(&q.lightning_payment_id)
    .bind(q.burn_txn_hash.as_ref().map(element_bytes))
    .bind(&q.last_error)
    .bind(q.expires_at.timestamp())
    .bind(q.created_at.timestamp())
    .bind(q.updated_at.timestamp())
    .execute(pool)
    .await;

    match res {
        Ok(_) => Ok(()),
        Err(sqlx::Error::Database(db)) if db.message().contains("payment_hash") => {
            Err(Error::DuplicatePaymentHash)
        }
        Err(sqlx::Error::Database(db)) if db.message().contains("note_commitment") => {
            Err(Error::DuplicateNoteCommitment)
        }
        Err(e) => Err(Error::Sqlx(e)),
    }
}

pub async fn get(pool: &DbPool, quote_id: Uuid) -> Result<Quote, Error> {
    let row = sqlx::query(
        r#"SELECT * FROM quotes WHERE quote_id = ?1"#,
    )
    .bind(quote_id.to_string())
    .fetch_optional(pool)
    .await?;

    let row = row.ok_or(Error::NotFound)?;
    row_to_quote(row)
}

pub async fn list_non_terminal(pool: &DbPool) -> Result<Vec<Quote>, Error> {
    let rows = sqlx::query(
        r#"
        SELECT * FROM quotes
        WHERE status NOT IN ('SlowBurnConfirmed', 'Refundable', 'Cancelled')
        ORDER BY created_at ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_quote).collect()
}

pub async fn update_status(
    pool: &DbPool,
    quote_id: Uuid,
    status: QuoteStatus,
) -> Result<(), Error> {
    sqlx::query(
        r#"UPDATE quotes SET status = ?1, updated_at = ?2 WHERE quote_id = ?3"#,
    )
    .bind(status.as_str())
    .bind(Utc::now().timestamp())
    .bind(quote_id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_payment_started(
    pool: &DbPool,
    quote_id: Uuid,
    payment_id: &str,
) -> Result<(), Error> {
    sqlx::query(
        r#"
        UPDATE quotes
        SET status = 'LightningPaying',
            lightning_payment_id = ?1,
            updated_at = ?2
        WHERE quote_id = ?3
        "#,
    )
    .bind(payment_id)
    .bind(Utc::now().timestamp())
    .bind(quote_id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_payment_succeeded(
    pool: &DbPool,
    quote_id: Uuid,
    preimage: [u8; 32],
) -> Result<(), Error> {
    sqlx::query(
        r#"
        UPDATE quotes
        SET status = 'LightningPaid',
            preimage = ?1,
            updated_at = ?2
        WHERE quote_id = ?3
        "#,
    )
    .bind(preimage.to_vec())
    .bind(Utc::now().timestamp())
    .bind(quote_id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_burn_submitted(
    pool: &DbPool,
    quote_id: Uuid,
    burn_txn_hash: Element,
) -> Result<(), Error> {
    sqlx::query(
        r#"
        UPDATE quotes
        SET status = 'SlowBurnSubmitted',
            burn_txn_hash = ?1,
            updated_at = ?2
        WHERE quote_id = ?3
        "#,
    )
    .bind(element_bytes(&burn_txn_hash))
    .bind(Utc::now().timestamp())
    .bind(quote_id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_error(pool: &DbPool, quote_id: Uuid, message: &str) -> Result<(), Error> {
    sqlx::query(
        r#"UPDATE quotes SET last_error = ?1, updated_at = ?2 WHERE quote_id = ?3"#,
    )
    .bind(message)
    .bind(Utc::now().timestamp())
    .bind(quote_id.to_string())
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

fn parse_status(s: &str) -> Result<QuoteStatus, Error> {
    QuoteStatus::from_str(s).map_err(Error::Row)
}

fn ts(secs: i64) -> Result<DateTime<Utc>, Error> {
    Utc.timestamp_opt(secs, 0)
        .single()
        .ok_or_else(|| Error::Row(format!("invalid timestamp {secs}")))
}

fn row_to_quote(row: sqlx::sqlite::SqliteRow) -> Result<Quote, Error> {
    let quote_id_str: String = row.try_get("quote_id")?;
    let quote_id = Uuid::parse_str(&quote_id_str)
        .map_err(|e| Error::Row(format!("invalid quote_id: {e}")))?;
    let status_str: String = row.try_get("status")?;
    let bolt11: String = row.try_get("bolt11")?;
    let payment_hash: Vec<u8> = row.try_get("payment_hash")?;
    let user_address: Vec<u8> = row.try_get("user_address")?;
    let note_kind: Vec<u8> = row.try_get("note_kind")?;
    let amount: Vec<u8> = row.try_get("amount")?;
    let zero_block: Vec<u8> = row.try_get("zero_block")?;
    let n_blocks: Vec<u8> = row.try_get("n_blocks")?;
    let note_commitment: Vec<u8> = row.try_get("note_commitment")?;
    let preimage: Option<Vec<u8>> = row.try_get("preimage")?;
    let lightning_payment_id: Option<String> = row.try_get("lightning_payment_id")?;
    let burn_txn_hash: Option<Vec<u8>> = row.try_get("burn_txn_hash")?;
    let last_error: Option<String> = row.try_get("last_error")?;
    let expires_at: i64 = row.try_get("expires_at")?;
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;

    Ok(Quote {
        quote_id,
        status: parse_status(&status_str)?,
        bolt11,
        payment_hash: fixed32(&payment_hash)?,
        user_address: element_from_bytes(&user_address)?,
        note_kind: element_from_bytes(&note_kind)?,
        amount: element_from_bytes(&amount)?,
        zero_block: fixed32(&zero_block)?,
        n_blocks: element_from_bytes(&n_blocks)?,
        note_commitment: element_from_bytes(&note_commitment)?,
        preimage: preimage.map(|b| fixed32(&b)).transpose()?,
        lightning_payment_id,
        burn_txn_hash: burn_txn_hash.map(|b| element_from_bytes(&b)).transpose()?,
        last_error,
        expires_at: ts(expires_at)?,
        created_at: ts(created_at)?,
        updated_at: ts(updated_at)?,
    })
}
