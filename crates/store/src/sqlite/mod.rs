//! A store in one SQLite file.

mod sanctions;
mod schema;

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use unsup_protocol::{OwnerId, ResidentId};

use crate::error::StoreError;
use crate::model::{NewOwner, NewResident, Owner, Principal, ResidentRecord, StoreCounts, Webhook};
use crate::sanction::{Sanction, SanctionRecord};
use crate::store::Store;
use crate::token::{Secret, TokenHash, TokenKind};

/// Milliseconds that a call waits for a lock of another connection.
const BUSY_TIMEOUT_MS: u32 = 5_000;

/// A store in one SQLite file. One connection serves all calls, one at a
/// time.
#[derive(Debug)]
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Opens the file at `path`, makes it when it does not exist, and
    /// brings its schema to the latest version.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be opened, when a migration fails, or
    /// when a newer build wrote the file.
    pub fn open(path: &Path) -> Result<SqliteStore, StoreError> {
        let conn = Connection::open(path)
            .map_err(|e| StoreError::Engine(format!("cannot open {}: {e}", path.display())))?;
        // The write-ahead log lets a reader of the file (a backup tool)
        // work while the server writes.
        conn.pragma_update(None, "journal_mode", "WAL").map_err(engine)?;
        conn.pragma_update(None, "synchronous", "NORMAL").map_err(engine)?;
        SqliteStore::prepare(conn)
    }

    /// Opens a database in memory. It is gone when the store is dropped.
    ///
    /// # Errors
    ///
    /// Fails when SQLite cannot make the database.
    pub fn open_in_memory() -> Result<SqliteStore, StoreError> {
        SqliteStore::prepare(Connection::open_in_memory().map_err(engine)?)
    }

    fn prepare(mut conn: Connection) -> Result<SqliteStore, StoreError> {
        conn.pragma_update(None, "foreign_keys", "ON").map_err(engine)?;
        conn.pragma_update(None, "busy_timeout", BUSY_TIMEOUT_MS).map_err(engine)?;
        schema::migrate(&mut conn)?;
        Ok(SqliteStore { conn: Mutex::new(conn) })
    }

    /// Returns the schema version of the file. After a successful open it
    /// is the latest version that this build knows.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read.
    pub fn schema_version(&self) -> Result<u32, StoreError> {
        schema::version(&self.lock())
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        // A transaction that a panic left open is rolled back when its
        // guard drops, so the connection behind a poisoned lock is good.
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `work` in one transaction.
    fn transact<T>(
        &self,
        work: impl FnOnce(&Transaction<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(engine)?;
        let value = work(&tx)?;
        tx.commit().map_err(engine)?;
        Ok(value)
    }
}

/// Converts an engine error that no caller can act on.
fn engine(error: rusqlite::Error) -> StoreError {
    StoreError::Engine(error.to_string())
}

/// Converts an error of an insert: a broken uniqueness rule is a conflict.
fn insert_error(what: &str) -> impl Fn(rusqlite::Error) -> StoreError + '_ {
    move |error| match &error {
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            StoreError::Conflict(what.to_string())
        }
        _ => engine(error),
    }
}

/// Converts a number to the integer type of SQLite.
fn int(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::Engine(format!("the number {value} is too large")))
}

/// Reads a non-negative integer column.
fn uint(row: &Row<'_>, column: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(column)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
}

fn owner_row(row: &Row<'_>) -> rusqlite::Result<Owner> {
    Ok(Owner {
        id: OwnerId(uint(row, 0)?),
        name: row.get(1)?,
        provider: row.get(2)?,
        subject: row.get(3)?,
        created_at: uint(row, 4)?,
    })
}

const OWNER_COLUMNS: &str = "id, name, provider, subject, created_at";

fn resident_row(row: &Row<'_>) -> rusqlite::Result<ResidentRecord> {
    Ok(ResidentRecord {
        resident: ResidentId(uint(row, 0)?),
        owner: OwnerId(uint(row, 1)?),
        name: row.get(2)?,
        model: row.get(3)?,
        created_at: uint(row, 4)?,
        key_hint: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
    })
}

/// A query for live resident records with the hint of their key. Add a
/// condition on `r` after it.
const RESIDENT_QUERY: &str = "
    SELECT r.resident, r.owner, r.name, r.model, r.created_at, t.hint
    FROM residents r LEFT JOIN tokens t ON t.record = r.record
    WHERE r.gone_at IS NULL";

fn webhook_row(row: &Row<'_>) -> rusqlite::Result<Webhook> {
    Ok(Webhook {
        resident: ResidentId(uint(row, 0)?),
        url: row.get(1)?,
        secret: Secret::from_stored(row.get(2)?),
        created_at: uint(row, 3)?,
    })
}

const WEBHOOK_QUERY: &str = "
    SELECT r.resident, w.url, w.secret, w.created_at
    FROM webhooks w JOIN residents r ON r.record = w.record";

/// Returns the row id of the live record of a resident.
fn live_record(tx: &Transaction<'_>, resident: ResidentId) -> Result<Option<i64>, StoreError> {
    tx.query_row(
        "SELECT record FROM residents WHERE resident = ?1 AND gone_at IS NULL",
        [int(resident.0)?],
        |row| row.get(0),
    )
    .optional()
    .map_err(engine)
}

fn no_resident(resident: ResidentId) -> StoreError {
    StoreError::NotFound(format!("resident {resident}"))
}

fn insert_token(
    tx: &Transaction<'_>,
    token: &TokenHash,
    kind: TokenKind,
    owner: OwnerId,
    record: Option<i64>,
    hint: &str,
    now: u64,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT INTO tokens (hash, kind, owner, record, hint, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![token.as_bytes().as_slice(), kind.as_str(), int(owner.0)?, record, hint, int(now)?],
    )
    .map_err(insert_error("the token exists already"))?;
    Ok(())
}

impl Store for SqliteStore {
    fn create_owner(
        &self,
        new: &NewOwner,
        token: &TokenHash,
        hint: &str,
        now: u64,
    ) -> Result<Owner, StoreError> {
        self.transact(|tx| {
            tx.execute(
                "INSERT INTO owners (name, provider, subject, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![new.name, new.provider, new.subject, int(now)?],
            )
            .map_err(|e| {
                let what = format!("an owner of the provider `{}` with this subject", new.provider);
                insert_error(&what)(e)
            })?;
            let id = OwnerId(
                u64::try_from(tx.last_insert_rowid())
                    .map_err(|_| StoreError::Engine("SQLite gave a negative owner id".into()))?,
            );
            insert_token(tx, token, TokenKind::Owner, id, None, hint, now)?;
            Ok(Owner {
                id,
                name: new.name.clone(),
                provider: new.provider.clone(),
                subject: new.subject.clone(),
                created_at: now,
            })
        })
    }

    fn owner(&self, id: OwnerId) -> Result<Option<Owner>, StoreError> {
        self.lock()
            .query_row(
                &format!("SELECT {OWNER_COLUMNS} FROM owners WHERE id = ?1"),
                [int(id.0)?],
                owner_row,
            )
            .optional()
            .map_err(engine)
    }

    fn owner_by_identity(
        &self,
        provider: &str,
        subject: &str,
    ) -> Result<Option<Owner>, StoreError> {
        self.lock()
            .query_row(
                &format!("SELECT {OWNER_COLUMNS} FROM owners WHERE provider = ?1 AND subject = ?2"),
                [provider, subject],
                owner_row,
            )
            .optional()
            .map_err(engine)
    }

    fn add_owner_token(
        &self,
        owner: OwnerId,
        token: &TokenHash,
        hint: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        self.transact(|tx| {
            let exists: Option<i64> = tx
                .query_row("SELECT id FROM owners WHERE id = ?1", [int(owner.0)?], |row| row.get(0))
                .optional()
                .map_err(engine)?;
            if exists.is_none() {
                return Err(StoreError::NotFound(format!("owner {owner}")));
            }
            insert_token(tx, token, TokenKind::Owner, owner, None, hint, now)
        })
    }

    fn authenticate(&self, token: &TokenHash) -> Result<Option<Principal>, StoreError> {
        let conn = self.lock();
        let found = conn
            .query_row(
                "SELECT t.hash, t.kind, t.owner, r.resident
                 FROM tokens t LEFT JOIN residents r ON r.record = t.record AND r.gone_at IS NULL
                 WHERE t.hash = ?1",
                [token.as_bytes().as_slice()],
                |row| {
                    let hash: Vec<u8> = row.get(0)?;
                    let kind: String = row.get(1)?;
                    let owner = OwnerId(uint(row, 2)?);
                    let resident: Option<i64> = row.get(3)?;
                    Ok((hash, kind, owner, resident))
                },
            )
            .optional()
            .map_err(engine)?;
        let Some((hash, kind, owner, resident)) = found else {
            return Ok(None);
        };
        // The index finds the row; the constant-time check confirms it.
        if !TokenHash::from_bytes(&hash).is_some_and(|stored| stored.matches(token)) {
            return Ok(None);
        }
        let resident = resident.and_then(|id| u64::try_from(id).ok()).map(ResidentId);
        Ok(match (kind.as_str(), resident) {
            ("owner", _) => Some(Principal::Owner(owner)),
            ("resident", Some(resident)) => Some(Principal::Resident { resident, owner }),
            _ => None,
        })
    }

    fn create_resident(
        &self,
        new: &NewResident,
        key: &TokenHash,
        hint: &str,
        now: u64,
    ) -> Result<ResidentRecord, StoreError> {
        self.transact(|tx| {
            let owner: Option<i64> = tx
                .query_row("SELECT id FROM owners WHERE id = ?1", [int(new.owner.0)?], |row| {
                    row.get(0)
                })
                .optional()
                .map_err(engine)?;
            if owner.is_none() {
                return Err(StoreError::NotFound(format!("owner {}", new.owner)));
            }
            tx.execute(
                "INSERT INTO residents (resident, owner, name, model, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![int(new.resident.0)?, int(new.owner.0)?, new.name, new.model, int(now)?],
            )
            .map_err(|e| insert_error(&format!("resident {}", new.resident))(e))?;
            let record = tx.last_insert_rowid();
            insert_token(tx, key, TokenKind::Resident, new.owner, Some(record), hint, now)?;
            Ok(ResidentRecord {
                resident: new.resident,
                owner: new.owner,
                name: new.name.clone(),
                model: new.model.clone(),
                created_at: now,
                key_hint: hint.to_string(),
            })
        })
    }

    fn resident(&self, resident: ResidentId) -> Result<Option<ResidentRecord>, StoreError> {
        self.lock()
            .query_row(
                &format!("{RESIDENT_QUERY} AND r.resident = ?1"),
                [int(resident.0)?],
                resident_row,
            )
            .optional()
            .map_err(engine)
    }

    fn residents_of(&self, owner: OwnerId) -> Result<Vec<ResidentRecord>, StoreError> {
        let conn = self.lock();
        let mut statement = conn
            .prepare_cached(&format!("{RESIDENT_QUERY} AND r.owner = ?1 ORDER BY r.resident"))
            .map_err(engine)?;
        let rows = statement.query_map([int(owner.0)?], resident_row).map_err(engine)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(engine)
    }

    fn count_residents(&self, owner: OwnerId) -> Result<u64, StoreError> {
        self.lock()
            .query_row(
                "SELECT COUNT(*) FROM residents WHERE owner = ?1 AND gone_at IS NULL",
                [int(owner.0)?],
                |row| uint(row, 0),
            )
            .map_err(engine)
    }

    fn resident_ids(&self) -> Result<Vec<ResidentId>, StoreError> {
        let conn = self.lock();
        let mut statement = conn
            .prepare("SELECT resident FROM residents WHERE gone_at IS NULL ORDER BY resident")
            .map_err(engine)?;
        let rows = statement.query_map([], |row| uint(row, 0).map(ResidentId)).map_err(engine)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(engine)
    }

    fn rotate_resident_key(
        &self,
        resident: ResidentId,
        key: &TokenHash,
        hint: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        self.transact(|tx| {
            let record = live_record(tx, resident)?.ok_or_else(|| no_resident(resident))?;
            let owner: i64 = tx
                .query_row("SELECT owner FROM residents WHERE record = ?1", [record], |row| {
                    row.get(0)
                })
                .map_err(engine)?;
            let owner = OwnerId(u64::try_from(owner).map_err(|_| {
                StoreError::Engine("the database holds a negative owner id".into())
            })?);
            tx.execute("DELETE FROM tokens WHERE record = ?1", [record]).map_err(engine)?;
            insert_token(tx, key, TokenKind::Resident, owner, Some(record), hint, now)
        })
    }

    fn retire_resident(
        &self,
        resident: ResidentId,
        reason: &str,
        now: u64,
    ) -> Result<bool, StoreError> {
        self.transact(|tx| {
            let Some(record) = live_record(tx, resident)? else {
                return Ok(false);
            };
            tx.execute(
                "UPDATE residents SET gone_at = ?1, gone_reason = ?2 WHERE record = ?3",
                params![int(now)?, reason, record],
            )
            .map_err(engine)?;
            tx.execute("DELETE FROM tokens WHERE record = ?1", [record]).map_err(engine)?;
            tx.execute("DELETE FROM webhooks WHERE record = ?1", [record]).map_err(engine)?;
            sanctions::end_for_resident(tx, resident)?;
            Ok(true)
        })
    }

    fn retire_all_residents(&self, reason: &str, now: u64) -> Result<u64, StoreError> {
        self.transact(|tx| {
            // Keys and webhooks belong to live records only, so all go.
            tx.execute("DELETE FROM tokens WHERE record IS NOT NULL", []).map_err(engine)?;
            tx.execute("DELETE FROM webhooks", []).map_err(engine)?;
            sanctions::end_for_all_residents(tx)?;
            let changed = tx
                .execute(
                    "UPDATE residents SET gone_at = ?1, gone_reason = ?2 WHERE gone_at IS NULL",
                    params![int(now)?, reason],
                )
                .map_err(engine)?;
            Ok(changed as u64)
        })
    }

    fn impose(&self, sanction: Sanction, reason: &str, now: u64) -> Result<bool, StoreError> {
        self.transact(|tx| {
            match sanction {
                Sanction::Mute(resident) | Sanction::Freeze(resident) => {
                    live_record(tx, resident)?.ok_or_else(|| no_resident(resident))?;
                }
                Sanction::Ban(owner) => {
                    let exists: Option<i64> = tx
                        .query_row("SELECT id FROM owners WHERE id = ?1", [int(owner.0)?], |row| {
                            row.get(0)
                        })
                        .optional()
                        .map_err(engine)?;
                    if exists.is_none() {
                        return Err(StoreError::NotFound(format!("owner {owner}")));
                    }
                }
            }
            sanctions::impose(tx, sanction, reason, now)
        })
    }

    fn lift(&self, sanction: Sanction) -> Result<bool, StoreError> {
        sanctions::lift(&self.lock(), sanction)
    }

    fn sanctions(&self) -> Result<Vec<SanctionRecord>, StoreError> {
        sanctions::list(&self.lock())
    }

    fn set_webhook(
        &self,
        resident: ResidentId,
        url: &str,
        secret: &Secret,
        now: u64,
    ) -> Result<(), StoreError> {
        self.transact(|tx| {
            let record = live_record(tx, resident)?.ok_or_else(|| no_resident(resident))?;
            tx.execute(
                "INSERT INTO webhooks (record, url, secret, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (record) DO UPDATE SET
                     url = excluded.url, secret = excluded.secret, created_at = excluded.created_at",
                params![record, url, secret.reveal(), int(now)?],
            )
            .map_err(engine)?;
            Ok(())
        })
    }

    fn clear_webhook(&self, resident: ResidentId) -> Result<bool, StoreError> {
        self.transact(|tx| {
            let Some(record) = live_record(tx, resident)? else {
                return Ok(false);
            };
            let changed =
                tx.execute("DELETE FROM webhooks WHERE record = ?1", [record]).map_err(engine)?;
            Ok(changed > 0)
        })
    }

    fn webhook(&self, resident: ResidentId) -> Result<Option<Webhook>, StoreError> {
        self.lock()
            .query_row(
                &format!("{WEBHOOK_QUERY} WHERE r.resident = ?1 AND r.gone_at IS NULL"),
                [int(resident.0)?],
                webhook_row,
            )
            .optional()
            .map_err(engine)
    }

    fn webhooks(&self) -> Result<Vec<Webhook>, StoreError> {
        let conn = self.lock();
        let mut statement = conn
            .prepare(&format!("{WEBHOOK_QUERY} WHERE r.gone_at IS NULL ORDER BY r.resident"))
            .map_err(engine)?;
        let rows = statement.query_map([], webhook_row).map_err(engine)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(engine)
    }

    fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.lock()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| row.get(0))
            .optional()
            .map_err(engine)
    }

    fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                [key, value],
            )
            .map_err(engine)?;
        Ok(())
    }

    fn clear_setting(&self, key: &str) -> Result<bool, StoreError> {
        let changed =
            self.lock().execute("DELETE FROM settings WHERE key = ?1", [key]).map_err(engine)?;
        Ok(changed > 0)
    }

    fn counts(&self) -> Result<StoreCounts, StoreError> {
        let conn = self.lock();
        let count = |sql: &str| conn.query_row(sql, [], |row| uint(row, 0)).map_err(engine);
        Ok(StoreCounts {
            owners: count("SELECT COUNT(*) FROM owners")?,
            residents: count("SELECT COUNT(*) FROM residents WHERE gone_at IS NULL")?,
            retired: count("SELECT COUNT(*) FROM residents WHERE gone_at IS NOT NULL")?,
            webhooks: count("SELECT COUNT(*) FROM webhooks")?,
        })
    }
}
