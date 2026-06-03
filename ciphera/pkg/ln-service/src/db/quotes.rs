use crate::db::DbPool;
use crate::domain::{Quote, QuoteStatus};
use chrono::{DateTime, TimeZone, Utc};
use element::Element;
use sqlx::Row;
use std::str::FromStr;

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

/// Insert a fresh quote. The column list, placeholder count (`?1..?15`),
/// and bind order all have to agree -- the previous version was
/// silently dropping `user_address`, `preimage`, `claim_address` and
/// double-binding `payment_hash` into the `zero_block` slot. Keep this
/// strictly aligned with the `CREATE TABLE quotes (...)` migration in
/// `db/mod.rs`.
pub async fn insert(pool: &DbPool, q: &Quote) -> Result<(), Error> {
    let res = sqlx::query(
        r#"
        INSERT INTO quotes (
            payment_hash,
            bolt11,
            preimage,
            note_commitment,
            note_kind,
            note_secret,
            amount,
            user_address,
            zero_block,
            n_blocks,
            claim_address,
            status,
            last_error,
            expires_at,
            created_at,
            updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16);
        "#,
    )
    .bind(&q.payment_hash[..])
    .bind(&q.bolt11)
    .bind(q.preimage.as_ref().map(|p| p.to_vec()))
    .bind(element_bytes(&q.note_commitment))
    .bind(element_bytes(&q.note_kind))
    .bind(element_bytes(&q.note_secret))
    .bind(element_bytes(&q.amount))
    .bind(element_bytes(&q.user_address))
    .bind(&q.zero_block[..])
    .bind(element_bytes(&q.n_blocks))
    .bind(q.claim_address.as_ref().map(element_bytes))
    .bind(q.status.as_str())
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

pub async fn get(pool: &DbPool, payment_hash: [u8; 32]) -> Result<Quote, Error> {
    let row = sqlx::query(r#"SELECT * FROM quotes WHERE payment_hash = ?1"#)
        .bind(&payment_hash[..])
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or(Error::NotFound)?;
    row_to_quote(row)
}

/// Quotes the settlement worker should still tick. Anything in
/// `is_terminal()` (`ClaimConfirmed`, `Refundable`, `Cancelled`) is
/// excluded so the worker doesn't keep waking on finished rows.
pub async fn list_non_terminal(pool: &DbPool) -> Result<Vec<Quote>, Error> {
    let rows = sqlx::query(
        r#"
        SELECT * FROM quotes
        WHERE status NOT IN ('ClaimConfirmed', 'Refundable', 'Cancelled')
        ORDER BY created_at ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_quote).collect()
}

pub async fn update_status(
    pool: &DbPool,
    payment_hash: [u8; 32],
    status: QuoteStatus,
) -> Result<(), Error> {
    sqlx::query(r#"UPDATE quotes SET status = ?1, updated_at = ?2 WHERE payment_hash = ?3"#)
        .bind(status.as_str())
        .bind(Utc::now().timestamp())
        .bind(&payment_hash[..])
        .execute(pool)
        .await?;
    Ok(())
}

/// Mark the quote as `LightningPaying`. Placeholder numbering used to
/// start at `?2` with two bind values, which would have produced a
/// runtime sqlite error.
pub async fn record_payment_started(pool: &DbPool, payment_hash: [u8; 32]) -> Result<(), Error> {
    sqlx::query(
        r#"
        UPDATE quotes
        SET status = 'LightningPaying',
            updated_at = ?1
        WHERE payment_hash = ?2
        "#,
    )
    .bind(Utc::now().timestamp())
    .bind(&payment_hash[..])
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_payment_succeeded(
    pool: &DbPool,
    payment_hash: [u8; 32],
    preimage: [u8; 32],
) -> Result<(), Error> {
    sqlx::query(
        r#"
        UPDATE quotes
        SET status = 'LightningPaid',
            preimage = ?1,
            updated_at = ?2
        WHERE payment_hash = ?3
        "#,
    )
    .bind(preimage.to_vec())
    .bind(Utc::now().timestamp())
    .bind(&payment_hash[..])
    .execute(pool)
    .await?;
    Ok(())
}

/// Stamp the ciphera tx hash returned by `submit_escrow_transaction` and
/// transition to `ClaimSubmitted`. Previous WHERE clause referenced
/// `quote_id`, which doesn't exist in the schema -- updates silently
/// no-op'd on every row.
pub async fn record_claim_submitted(
    pool: &DbPool,
    payment_hash: [u8; 32],
    claim_address: Element,
) -> Result<(), Error> {
    sqlx::query(
        r#"
        UPDATE quotes
        SET status = 'ClaimSubmitted',
            claim_address = ?1,
            updated_at = ?2
        WHERE payment_hash = ?3
        "#,
    )
    .bind(element_bytes(&claim_address))
    .bind(Utc::now().timestamp())
    .bind(&payment_hash[..])
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_error(
    pool: &DbPool,
    payment_hash: [u8; 32],
    message: &str,
) -> Result<(), Error> {
    sqlx::query(r#"UPDATE quotes SET last_error = ?1, updated_at = ?2 WHERE payment_hash = ?3"#)
        .bind(message)
        .bind(Utc::now().timestamp())
        .bind(&payment_hash[..])
        .execute(pool)
        .await?;
    Ok(())
}

fn parse_status(s: &str) -> Result<QuoteStatus, Error> {
    QuoteStatus::from_str(s).map_err(Error::Row)
}

fn element_bytes(e: &Element) -> Vec<u8> {
    e.to_be_bytes().to_vec()
}

fn element_from_bytes(b: &[u8]) -> Result<Element, Error> {
    if b.len() != 32 {
        return Err(Error::Row(format!(
            "expected 32-byte Element, got {}",
            b.len()
        )));
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

fn row_to_quote(row: sqlx::sqlite::SqliteRow) -> Result<Quote, Error> {
    let payment_hash: Vec<u8> = row.try_get("payment_hash")?;
    let bolt11: String = row.try_get("bolt11")?;
    let preimage: Option<Vec<u8>> = row.try_get("preimage")?;
    let note_commitment: Vec<u8> = row.try_get("note_commitment")?;
    let user_address: Vec<u8> = row.try_get("user_address")?;
    let note_kind: Vec<u8> = row.try_get("note_kind")?;
    let note_secret: Vec<u8> = row.try_get("note_secret")?;
    let amount: Vec<u8> = row.try_get("amount")?;
    let zero_block: Vec<u8> = row.try_get("zero_block")?;
    let n_blocks: Vec<u8> = row.try_get("n_blocks")?;
    let claim_address: Option<Vec<u8>> = row.try_get("claim_address")?;
    let status_str: String = row.try_get("status")?;
    let last_error: Option<String> = row.try_get("last_error")?;
    let expires_at: i64 = row.try_get("expires_at")?;
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;

    Ok(Quote {
        payment_hash: fixed32(&payment_hash)?,
        bolt11,
        preimage: preimage.map(|b| fixed32(&b)).transpose()?,
        note_commitment: element_from_bytes(&note_commitment)?,
        note_kind: element_from_bytes(&note_kind)?,
        user_address: element_from_bytes(&user_address)?,
        note_secret: element_from_bytes(&note_secret)?,
        amount: element_from_bytes(&amount)?,
        zero_block: fixed32(&zero_block)?,
        n_blocks: element_from_bytes(&n_blocks)?,
        claim_address: claim_address.map(|b| element_from_bytes(&b)).transpose()?,
        status: parse_status(&status_str)?,
        last_error,
        expires_at: ts(expires_at)?,
        created_at: ts(created_at)?,
        updated_at: ts(updated_at)?,
    })
}
