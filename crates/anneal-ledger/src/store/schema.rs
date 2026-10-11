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

/// What a file holds before the ledger changes anything in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Contents {
    /// No schema objects: a new file or an empty database, which gets the ledger schema.
    Empty,
    /// happenstance's schema (its `store_meta` table).
    Ledger,
    /// Schema objects that are not happenstance's: someone else's database.
    Foreign,
}

/// Opens (creating when absent) the ledger at `path` and hands its connection to happenstance.
///
/// The order matters: until the file is known to be this build's ledger or an empty database,
/// the connection only reads, so a file that is refused (newer schema, WAL, not a ledger) is
/// left exactly as it was. A new file gets its schema and stamp in one `BEGIN IMMEDIATE`.
pub(super) fn open(path: &Path) -> Result<SqliteEventStore, OpenError> {
    let conn = connect(path)?;
    let contents = inspect(&conn)?;
    use_truncate(&conn)?;
    if contents == Contents::Empty {
        create_schema(&conn)?;
    }
    match user_version(&conn)? {
        LEDGER_SCHEMA => SqliteEventStore::new(conn).map_err(OpenError::Store),
        found => Err(OpenError::UnknownSchema { found }),
    }
}

/// A connection to `path` with a busy timeout. Opening it changes nothing in the file.
pub(crate) fn connect(path: &Path) -> Result<Connection, OpenError> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    Ok(conn)
}

/// Reads what the file holds, refusing a ledger stamped by a newer build, a file in WAL and a
/// database that is not a ledger. Only reads.
///
/// Each read is its own statement, so another process may create the schema between them:
/// the version read here can be 0 for a file that is a ledger by the time `contents` looks.
/// [`open`] therefore checks the exact version again once the schema is in place.
fn inspect(conn: &Connection) -> Result<Contents, OpenError> {
    refuse_newer(user_version(conn)?)?;
    refuse_wal(conn)?;
    match contents(conn)? {
        Contents::Foreign => Err(OpenError::NotALedger),
        known => Ok(known),
    }
}

/// WAL is the one journal mode a file keeps between connections. A WAL file is refused, not
/// converted: another process may be using it in WAL.
fn refuse_wal(conn: &Connection) -> Result<(), OpenError> {
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    if mode.eq_ignore_ascii_case("wal") {
        Err(OpenError::JournalMode { found: mode })
    } else {
        Ok(())
    }
}

fn contents(conn: &Connection) -> Result<Contents, OpenError> {
    let (objects, ledger): (i64, i64) = conn.query_row(
        "SELECT count(*), count(*) FILTER (WHERE type = 'table' AND name = 'store_meta')
         FROM main.sqlite_master",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(match (objects, ledger) {
        (_, 1..) => Contents::Ledger,
        (0, _) => Contents::Empty,
        _ => Contents::Foreign,
    })
}

/// Puts the connection under [`JOURNAL_MODE`], with full syncs and foreign keys.
fn use_truncate(conn: &Connection) -> Result<(), OpenError> {
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
    Ok(())
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

/// Gives an empty database happenstance's schema and a fresh store identity without letting
/// happenstance open it (`SqliteEventStore::open` would switch the file to WAL). The schema is
/// built in a template on the local temporary disk and copied in.
fn create_schema(conn: &Connection) -> Result<(), OpenError> {
    let dir = tempfile::tempdir().map_err(OpenError::TempDir)?;
    let template = dir.path().join("template.sqlite3");
    drop(SqliteEventStore::open(&template).map_err(OpenError::Template)?);
    conn.execute("ATTACH DATABASE ?1 AS tpl", [attach_name(&template)?])?;
    let copied = copy_schema(conn);
    let detached = conn.execute_batch("DETACH DATABASE tpl");
    match (copied, detached) {
        (Ok(()), Ok(())) => {}
        (Err(failure), Ok(())) => return Err(failure),
        (Err(failure), Err(cleanup)) => {
            return Err(OpenError::Cleanup {
                failure: Box::new(failure),
                cleanup,
            });
        }
        (Ok(()), Err(detach)) => return Err(OpenError::Sqlite(detach)),
    }
    dir.close().map_err(OpenError::TempDir)
}

/// `path` as the UTF-8 text `ATTACH` takes.
fn attach_name(path: &Path) -> Result<&str, OpenError> {
    path.to_str().ok_or_else(|| OpenError::TemplatePath {
        path: path.to_path_buf(),
    })
}

/// Copies the template's tables, indexes and rows into `main` and stamps [`LEDGER_SCHEMA`], in
/// one `BEGIN IMMEDIATE`, so two processes creating the same ledger cannot both write a schema.
fn copy_schema(conn: &Connection) -> Result<(), OpenError> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let copied = copy_schema_objects(conn);
    finish_transaction(conn, copied)
}

fn copy_schema_objects(conn: &Connection) -> Result<(), OpenError> {
    // Another process may have created the schema, or written its own tables, since `inspect`.
    match contents(conn)? {
        Contents::Empty => {}
        Contents::Ledger => return Ok(()),
        Contents::Foreign => return Err(OpenError::NotALedger),
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
    if objects.is_empty() {
        return Err(OpenError::EmptyTemplate);
    }
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
    use std::path::Path;

    use super::{OpenError, attach_name, copy_schema, finish_transaction, user_version};
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
    fn an_empty_template_is_an_error_and_nothing_is_stamped() {
        let conn = Connection::open_in_memory().expect("in-memory database");
        conn.execute_batch("ATTACH DATABASE ':memory:' AS tpl")
            .expect("attaches an empty template");

        let result = copy_schema(&conn);

        assert!(
            matches!(result, Err(OpenError::EmptyTemplate)),
            "{result:?}"
        );
        assert_eq!(user_version(&conn).expect("readable"), 0, "not stamped");
        assert!(conn.is_autocommit(), "the transaction is rolled back");
    }

    #[test]
    #[cfg(unix)]
    fn a_template_path_that_is_not_utf8_is_an_error() {
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/template-\xff.sqlite3"));

        let result = attach_name(path);

        assert!(
            matches!(&result, Err(OpenError::TemplatePath { path: found }) if found == path),
            "{result:?}"
        );
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
