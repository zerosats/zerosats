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
          payment_hash          BLOB PRIMARY KEY,
          bolt11                TEXT NOT NULL,
          preimage              BLOB,
          note_commitment       BLOB NOT NULL UNIQUEY,
          note_kind             BLOB NOT NULL,
          note_secret           BLOB NOT NULL,
          amount                BLOB NOT NULL,
          user_address          BLOB NOT NULL,
          zero_block            BLOB NOT NULL,
          n_blocks              BLOB NOT NULL,
          claim_address         BLOB,
          status                TEXT NOT NULL,
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
