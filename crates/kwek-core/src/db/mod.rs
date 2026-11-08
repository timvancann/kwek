//! SQLite: the single source of truth for the UI. One connection behind a mutex; every
//! write batch is one transaction. Async callers go through [`Db::call`], which runs the
//! closure on the blocking pool.

pub mod read;
pub mod write;

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{Error, Result};

const MIGRATIONS: &[&str] = &[
    include_str!("schema.sql"),
    include_str!("migrate_002.sql"),
    include_str!("migrate_003.sql"),
];

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "OFF")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.with(migrate)?;
        Ok(db)
    }

    /// Synchronous access, for tests and short reads on non-async paths.
    pub fn with<R>(&self, f: impl FnOnce(&mut Connection) -> Result<R>) -> Result<R> {
        let mut c = self.conn.lock().map_err(|_| Error::Other("db mutex poisoned".into()))?;
        f(&mut c)
    }

    pub async fn call<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<R> + Send + 'static,
    ) -> Result<R> {
        let db = self.clone();
        tokio::task::spawn_blocking(move || db.with(f))
            .await
            .map_err(|e| Error::Other(format!("db task: {e}")))?
    }

    /// Wipes all synced data (logout). Keeps the schema.
    pub fn wipe(&self) -> Result<()> {
        self.with(|c| {
            c.execute_batch(
                "BEGIN;
                 DELETE FROM space; DELETE FROM message; DELETE FROM user; DELETE FROM membership;
                 DELETE FROM my_reaction; DELETE FROM space_read_state; DELETE FROM thread_read_state;
                 DELETE FROM account; DELETE FROM kv; DELETE FROM chunk; DELETE FROM embedding;
                 INSERT INTO message_fts(message_fts) VALUES ('rebuild');
                 INSERT INTO chunk_fts(chunk_fts) VALUES ('rebuild');
                 COMMIT;",
            )?;
            Ok(())
        })
    }
}

fn migrate(c: &mut Connection) -> Result<()> {
    let version: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = c.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

pub fn kv_get(c: &Connection, key: &str) -> Result<Option<String>> {
    Ok(c.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

pub fn kv_set(c: &Connection, key: &str, value: &str) -> Result<()> {
    c.execute(
        "INSERT INTO kv(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_and_has_fts5() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let v: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
            assert_eq!(v, MIGRATIONS.len() as i64);
            kv_set(c, "a", "1")?;
            kv_set(c, "a", "2")?;
            assert_eq!(kv_get(c, "a")?.as_deref(), Some("2"));
            Ok(())
        })
        .unwrap();
    }
}
