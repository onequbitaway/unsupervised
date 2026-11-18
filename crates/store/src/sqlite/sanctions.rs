//! Sanctions in the SQLite file.
//!
//! A sanction is one row of the table `settings`. The key is
//! `sanction:<kind>:<id>` and the value is `<created_at> <reason>`. So the
//! sanctions need no table of their own, and a file of an older build
//! takes them without a migration.

use rusqlite::{Connection, params};
use unsup_protocol::ResidentId;

use super::{engine, int};
use crate::error::StoreError;
use crate::sanction::{Sanction, SanctionRecord};

/// What the key of every sanction starts with.
const PREFIX: &str = "sanction:";
/// The first text after every key that starts with [`PREFIX`]: the same
/// text with the next character in the place of the colon.
const PREFIX_END: &str = "sanction;";

fn key(sanction: Sanction) -> String {
    format!("{PREFIX}{}:{}", sanction.kind(), sanction.subject())
}

/// Reads one row. Returns `None` for a row that this module did not write.
fn read(key: &str, value: &str) -> Option<SanctionRecord> {
    let (kind, subject) = key.strip_prefix(PREFIX)?.split_once(':')?;
    let sanction = Sanction::from_parts(kind, subject.parse().ok()?)?;
    let (created_at, reason) = value.split_once(' ')?;
    Some(SanctionRecord {
        sanction,
        reason: reason.to_string(),
        created_at: created_at.parse().ok()?,
    })
}

/// Writes the row of a sanction. Returns false when the row was there.
pub(super) fn impose(
    conn: &Connection,
    sanction: Sanction,
    reason: &str,
    now: u64,
) -> Result<bool, StoreError> {
    let changed = conn
        .execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
            params![key(sanction), format!("{} {reason}", int(now)?)],
        )
        .map_err(engine)?;
    Ok(changed > 0)
}

/// Removes the row of a sanction. Returns false when there was none.
pub(super) fn lift(conn: &Connection, sanction: Sanction) -> Result<bool, StoreError> {
    let changed =
        conn.execute("DELETE FROM settings WHERE key = ?1", [key(sanction)]).map_err(engine)?;
    Ok(changed > 0)
}

/// Returns every sanction, in the order of [`Sanction`].
pub(super) fn list(conn: &Connection) -> Result<Vec<SanctionRecord>, StoreError> {
    let mut statement = conn
        .prepare("SELECT key, value FROM settings WHERE key >= ?1 AND key < ?2")
        .map_err(engine)?;
    let rows = statement
        .query_map([PREFIX, PREFIX_END], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(engine)?;
    let mut records = Vec::new();
    for row in rows {
        let (key, value) = row.map_err(engine)?;
        let record = read(&key, &value).ok_or_else(|| {
            StoreError::Engine(format!("the setting `{key}` does not hold a sanction"))
        })?;
        records.push(record);
    }
    records.sort_by_key(|record| record.sanction);
    Ok(records)
}

/// Ends the mute and the freeze of one resident.
pub(super) fn end_for_resident(conn: &Connection, resident: ResidentId) -> Result<(), StoreError> {
    for sanction in [Sanction::Mute(resident), Sanction::Freeze(resident)] {
        lift(conn, sanction)?;
    }
    Ok(())
}

/// Ends every mute and every freeze. Bans stay.
pub(super) fn end_for_all_residents(conn: &Connection) -> Result<(), StoreError> {
    for kind in ["mute", "freeze"] {
        conn.execute(
            "DELETE FROM settings WHERE key >= ?1 AND key < ?2",
            [format!("{PREFIX}{kind}:"), format!("{PREFIX}{kind};")],
        )
        .map_err(engine)?;
    }
    Ok(())
}
