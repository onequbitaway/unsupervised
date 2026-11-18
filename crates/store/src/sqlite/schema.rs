//! The SQLite schema and its migrations.
//!
//! The file keeps the number of the last migration in `user_version`.
//! Opening a file runs every migration after that number, each in one
//! transaction. A migration is never changed after a release: a change to
//! the schema is a new entry at the end of [`MIGRATIONS`].

use rusqlite::Connection;

use crate::error::StoreError;

/// Every migration, in order. The index plus one is the schema version.
const MIGRATIONS: &[&str] = &[
    // 1: owners, residents, tokens, webhooks, settings.
    "
    CREATE TABLE owners (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        name       TEXT    NOT NULL,
        provider   TEXT    NOT NULL,
        subject    TEXT    NOT NULL,
        created_at INTEGER NOT NULL,
        UNIQUE (provider, subject)
    );

    -- One row for each resident that an owner made. `gone_at` is NULL
    -- while the resident is in the running world.
    CREATE TABLE residents (
        record      INTEGER PRIMARY KEY AUTOINCREMENT,
        resident    INTEGER NOT NULL,
        owner       INTEGER NOT NULL REFERENCES owners (id),
        name        TEXT    NOT NULL,
        model       TEXT,
        created_at  INTEGER NOT NULL,
        gone_at     INTEGER,
        gone_reason TEXT
    );
    CREATE UNIQUE INDEX residents_live ON residents (resident) WHERE gone_at IS NULL;
    CREATE INDEX residents_owner ON residents (owner) WHERE gone_at IS NULL;

    -- `hash` is the SHA-256 of the token. The token itself is never kept.
    CREATE TABLE tokens (
        hash       BLOB    PRIMARY KEY,
        kind       TEXT    NOT NULL CHECK (kind IN ('owner', 'resident')),
        owner      INTEGER NOT NULL REFERENCES owners (id),
        record     INTEGER REFERENCES residents (record),
        hint       TEXT    NOT NULL,
        created_at INTEGER NOT NULL
    ) WITHOUT ROWID;
    CREATE INDEX tokens_record ON tokens (record);

    CREATE TABLE webhooks (
        record     INTEGER PRIMARY KEY REFERENCES residents (record),
        url        TEXT    NOT NULL,
        secret     TEXT    NOT NULL,
        created_at INTEGER NOT NULL
    );

    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    ) WITHOUT ROWID;
    ",
];

/// Returns the schema version that this build writes.
pub(crate) fn latest_version() -> u32 {
    u32::try_from(MIGRATIONS.len()).unwrap_or(u32::MAX)
}

/// Returns the schema version of an open file.
pub(crate) fn version(conn: &Connection) -> Result<u32, StoreError> {
    conn.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
        .map_err(|e| StoreError::Engine(format!("cannot read the schema version: {e}")))
}

/// Brings an open file to the latest schema.
///
/// # Errors
///
/// Fails when a migration fails, or when a newer build wrote the file.
pub(crate) fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let current = version(conn)?;
    if current > latest_version() {
        return Err(StoreError::Engine(format!(
            "the database has schema version {current}, but this build knows only version {}; \
             use a newer build",
            latest_version()
        )));
    }
    for (number, sql) in (1u32..).zip(MIGRATIONS).skip(current as usize) {
        let failed = |e: rusqlite::Error| StoreError::Engine(format!("migration {number}: {e}"));
        let tx = conn.transaction().map_err(failed)?;
        tx.execute_batch(sql).map_err(failed)?;
        tx.pragma_update(None, "user_version", number).map_err(failed)?;
        tx.commit().map_err(failed)?;
    }
    Ok(())
}
