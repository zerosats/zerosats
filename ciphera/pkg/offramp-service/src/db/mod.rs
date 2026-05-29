pub mod quotes;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use std::str::FromStr;

pub type DbPool = Pool<Sqlite>;

pub async fn connect(path: &str) -> eyre::Result<DbPool> {
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{path}"))?
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

async fn migrate(pool: &DbPool) -> eyre::Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS quotes (
          payment_hash          TEXT PRIMARY KEY,
          note_commitment       TEXT NOT NULL UNIQUEY,
          status                TEXT NOT NULL,
          bolt11                TEXT NOT NULL,
          note_kind             BLOB NOT NULL,
          amount                BLOB NOT NULL,
          zero_block            BLOB NOT NULL,
          n_blocks              BLOB NOT NULL,
          preimage              BLOB,
          last_error            TEXT,
          expires_at            INTEGER NOT NULL,
          created_at            INTEGER NOT NULL,
          updated_at            INTEGER NOT NULL
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS quotes_status_idx ON quotes(status);")
        .execute(pool)
        .await?;

    Ok(())
}
