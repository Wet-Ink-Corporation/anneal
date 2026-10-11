//! Opening a ledger file: one connection under a TRUNCATE journal, happenstance's schema copied
//! in from a template, and the ledger's schema version in `PRAGMA user_version`.

use std::path::Path;
use std::time::Duration;

use happenstance_sqlite::event_store::SqliteEventStore;
use happenstance_sqlite::rusqlite::{self, Connection};

use super::OpenError;

/// The journal mode every ledger connection runs under, as `PRAGMA journal_mode` reports it.
pub(crate) const JOURNAL_MODE: &str = "truncate";

/// The ledger schema this build reads and writes, kept in `PRAGMA user_version`.
///
/// 1. happenstance 0.3.2's event store schema, under a TRUNCATE journal.
pub(crate) const LEDGER_SCHEMA: i64 = 1;

/// How long a connection waits for another process's lock before reporting busy. Appends hold
/// the write lock for one small insert, so this is only reached by a stuck writer.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// Opens (creating when absent) the ledger at `path` and hands its connection to happenstance.
///
/// The order matters: a ledger stamped by a newer build is refused before anything could
/// write to it, and a new file gets its schema and stamp in one `BEGIN IMMEDIATE`.
pub(super) fn open(path: &Path) -> Result<SqliteEventStore, OpenError> {
    let conn = connect(path)?;
    refuse_newer(user_version(&conn)?)?;
    ensure_schema(&conn)?;
    match user_version(&conn)? {
        LEDGER_SCHEMA => SqliteEventStore::new(conn).map_err(OpenError::Store),
        found => Err(OpenError::UnknownSchema { found }),
    }
}

/// A connection under [`JOURNAL_MODE`], with a busy timeout and full syncs.
pub(crate) fn connect(path: &Path) -> Result<Connection, OpenError> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    let mode: String = conn.query_row(
        &format!("PRAGMA journal_mode = {JOURNAL_MODE}"),
        [],
        |row| row.get(0),
    )?;
    if !mode.eq_ignore_ascii_case(JOURNAL_MODE) {
        return Err(OpenError::JournalMode { found: mode });
    }
    // FULL, not NORMAL: under a rollback journal NORMAL can lose the last commit on power loss.
    conn.execute_batch("PRAGMA synchronous = FULL; PRAGMA foreign_keys = ON;")?;
    Ok(conn)
}

fn refuse_newer(found: i64) -> Result<(), OpenError> {
    if found > LEDGER_SCHEMA {
        Err(OpenError::NewerSchema {
            found,
            supported: LEDGER_SCHEMA,
        })
    } else {
        Ok(())
    }
}

pub(crate) fn user_version(conn: &Connection) -> Result<i64, OpenError> {
    Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

fn has_schema(conn: &Connection) -> Result<bool, OpenError> {
    let tables: i64 = conn.query_row(
        "SELECT count(*) FROM main.sqlite_master WHERE type = 'table' AND name = 'store_meta'",
        [],
        |row| row.get(0),
    )?;
    Ok(tables > 0)
}

/// Gives a new ledger happenstance's schema and a fresh store identity without letting
/// happenstance open it (`SqliteEventStore::open` would switch the file to WAL). The schema is
/// built in a template on the local temporary disk and copied in.
fn ensure_schema(conn: &Connection) -> Result<(), OpenError> {
    if has_schema(conn)? {
        return Ok(());
    }
    let dir = tempfile::tempdir().map_err(OpenError::TempDir)?;
    let template = dir.path().join("template.sqlite3");
    drop(SqliteEventStore::open(&template).map_err(OpenError::Template)?);
    conn.execute(
        "ATTACH DATABASE ?1 AS tpl",
        [template.to_string_lossy().as_ref()],
    )?;
    let copied = copy_schema(conn);
    let detached = conn.execute_batch("DETACH DATABASE tpl");
    match (copied, detached) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(failure), Ok(())) => Err(failure),
        (Err(failure), Err(cleanup)) => Err(OpenError::Cleanup {
            failure: Box::new(failure),
            cleanup,
        }),
        (Ok(()), Err(detach)) => Err(OpenError::Sqlite(detach)),
    }
}

/// Copies the template's tables, indexes and rows into `main` and stamps [`LEDGER_SCHEMA`], in
/// one `BEGIN IMMEDIATE`, so two processes creating the same ledger cannot both write a schema.
fn copy_schema(conn: &Connection) -> Result<(), OpenError> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let copied = copy_schema_objects(conn);
    finish_transaction(conn, copied)
}

fn copy_schema_objects(conn: &Connection) -> Result<(), OpenError> {
    // Another process may have created the schema while the template was built.
    if has_schema(conn)? {
        return Ok(());
    }
    let mut statement = conn.prepare(
        "SELECT type, name, sql FROM tpl.sqlite_master
         WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'
         ORDER BY CASE type WHEN 'table' THEN 0 ELSE 1 END, rowid",
    )?;
    let objects = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Unqualified DDL creates its object in `main`.
    for (_, _, sql) in &objects {
        conn.execute_batch(sql)?;
    }
    for (_, name, _) in objects.iter().filter(|(kind, _, _)| kind == "table") {
        let name = name.replace('"', "\"\"");
        conn.execute(
            &format!("INSERT INTO main.\"{name}\" SELECT * FROM tpl.\"{name}\""),
            [],
        )?;
    }
    conn.execute_batch(&format!("PRAGMA user_version = {LEDGER_SCHEMA}"))?;
    Ok(())
}

/// Ends the transaction a `BEGIN IMMEDIATE` opened: commits when `body` succeeded, and rolls
/// back when it failed or when the commit itself failed (a failed `COMMIT` leaves the
/// transaction open).
fn finish_transaction(conn: &Connection, body: Result<(), OpenError>) -> Result<(), OpenError> {
    let failure = match body {
        Ok(()) => match conn.execute_batch("COMMIT") {
            Ok(()) => return Ok(()),
            Err(commit) => OpenError::Sqlite(commit),
        },
        Err(failure) => failure,
    };
    // SQLite ends the transaction itself on some errors (SQLITE_FULL, IOERR, NOMEM); a ROLLBACK
    // then would only add a misleading "no transaction is active".
    if conn.is_autocommit() {
        return Err(failure);
    }
    match conn.execute_batch("ROLLBACK") {
        Ok(()) => Err(failure),
        Err(cleanup) => Err(OpenError::Cleanup {
            failure: Box::new(failure),
            cleanup,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{OpenError, finish_transaction};
    use happenstance_sqlite::rusqlite::Connection;

    /// A connection inside a transaction holding a row that breaks a deferred foreign key, so
    /// its `COMMIT` fails and leaves the transaction open.
    fn transaction_whose_commit_fails() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory database");
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE parent (id INTEGER PRIMARY KEY);
             CREATE TABLE child (parent INTEGER REFERENCES parent (id) DEFERRABLE INITIALLY DEFERRED);
             BEGIN IMMEDIATE;
             INSERT INTO child (parent) VALUES (1);",
        )
        .expect("set up");
        conn
    }

    #[test]
    #[cfg_attr(miri, ignore = "opens SQLite through FFI")]
    fn a_failed_commit_is_rolled_back_and_reported() {
        let conn = transaction_whose_commit_fails();

        let result = finish_transaction(&conn, Ok(()));

        assert!(matches!(result, Err(OpenError::Sqlite(_))), "{result:?}");
        assert!(conn.is_autocommit(), "the transaction is rolled back");
        let children: i64 = conn
            .query_row("SELECT count(*) FROM child", [], |row| row.get(0))
            .expect("countable");
        assert_eq!(children, 0, "the rolled-back row is gone");
    }

    #[test]
    #[cfg_attr(miri, ignore = "opens SQLite through FFI")]
    fn a_failed_body_is_rolled_back_and_its_error_returned() {
        let conn = transaction_whose_commit_fails();

        let result = finish_transaction(&conn, Err(OpenError::UnknownSchema { found: 7 }));

        assert!(
            matches!(result, Err(OpenError::UnknownSchema { found: 7 })),
            "{result:?}"
        );
        assert!(conn.is_autocommit(), "the transaction is rolled back");
    }
}
