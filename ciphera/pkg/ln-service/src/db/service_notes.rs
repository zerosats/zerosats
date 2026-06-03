use crate::db::DbPool;
use crate::domain::ServiceNote;
use chrono::{DateTime, TimeZone, Utc};
use element::Element;
use sqlx::Row;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid row: {0}")]
    Row(String),
}

/// Record a service-owned note. Idempotent on `commitment` (re-recording
/// a claim that already landed is a no-op) via `INSERT OR IGNORE`.
pub async fn insert(pool: &DbPool, n: &ServiceNote) -> Result<(), Error> {
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO service_notes (
            commitment,
            note_secret,
            note_kind,
            value,
            spent,
            source_payment_hash,
            created_at,
            updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8);
        "#,
    )
    .bind(element_bytes(&n.commitment))
    .bind(element_bytes(&n.note_secret))
    .bind(element_bytes(&n.note_kind))
    .bind(value_to_i64(n.value)?)
    .bind(i64::from(n.spent))
    .bind(n.source_payment_hash.as_ref().map(|p| p.to_vec()))
    .bind(n.created_at.timestamp())
    .bind(n.updated_at.timestamp())
    .execute(pool)
    .await?;
    Ok(())
}

/// Pick the smallest unspent note of `note_kind` that covers `min_value`
/// wei (best-fit, to minimise change). `None` if the service has
/// insufficient liquidity.
pub async fn select_available(
    pool: &DbPool,
    note_kind: Element,
    min_value: u64,
) -> Result<Option<ServiceNote>, Error> {
    let row = sqlx::query(
        r#"
        SELECT * FROM service_notes
        WHERE spent = 0 AND note_kind = ?1 AND value >= ?2
        ORDER BY value ASC
        LIMIT 1
        "#,
    )
    .bind(element_bytes(&note_kind))
    .bind(value_to_i64(min_value)?)
    .fetch_optional(pool)
    .await?;
    row.map(row_to_service_note).transpose()
}

/// Mark a note spent. Returns the number of rows updated so callers can
/// detect a race where the note was already consumed.
pub async fn mark_spent(pool: &DbPool, commitment: Element) -> Result<u64, Error> {
    let res = sqlx::query(
        r#"UPDATE service_notes SET spent = 1, updated_at = ?1 WHERE commitment = ?2 AND spent = 0"#,
    )
    .bind(Utc::now().timestamp())
    .bind(element_bytes(&commitment))
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
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

fn value_to_i64(v: u64) -> Result<i64, Error> {
    i64::try_from(v).map_err(|_| Error::Row(format!("note value {v} exceeds i64::MAX")))
}

fn ts(secs: i64) -> Result<DateTime<Utc>, Error> {
    Utc.timestamp_opt(secs, 0)
        .single()
        .ok_or_else(|| Error::Row(format!("invalid timestamp {secs}")))
}

fn row_to_service_note(row: sqlx::sqlite::SqliteRow) -> Result<ServiceNote, Error> {
    let commitment: Vec<u8> = row.try_get("commitment")?;
    let note_secret: Vec<u8> = row.try_get("note_secret")?;
    let note_kind: Vec<u8> = row.try_get("note_kind")?;
    let value: i64 = row.try_get("value")?;
    let spent: i64 = row.try_get("spent")?;
    let source_payment_hash: Option<Vec<u8>> = row.try_get("source_payment_hash")?;
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;

    Ok(ServiceNote {
        commitment: element_from_bytes(&commitment)?,
        note_secret: element_from_bytes(&note_secret)?,
        note_kind: element_from_bytes(&note_kind)?,
        value: u64::try_from(value)
            .map_err(|_| Error::Row(format!("negative note value {value}")))?,
        spent: spent != 0,
        source_payment_hash: source_payment_hash.map(|b| fixed32(&b)).transpose()?,
        created_at: ts(created_at)?,
        updated_at: ts(updated_at)?,
    })
}
