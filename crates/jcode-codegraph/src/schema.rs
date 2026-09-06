//! SQLite DDL for the code graph (plan section 4 + pass-2 edge cases).
//!
//! Co-change pairs stored canonically (`a_id < b_id`, query both directions).
//! Self-deps skipped at insert. `PRAGMA user_version` gates wipe-and-rebuild.

use anyhow::Result;
use rusqlite::Connection;

/// Current schema version. Bump on any DDL change; `ensure_schema` wipes.
pub const SCHEMA_VERSION: i32 = 1;

const DDL: &str = "
CREATE TABLE IF NOT EXISTS files(
  id INTEGER PRIMARY KEY,
  path TEXT UNIQUE NOT NULL,
  language TEXT NOT NULL,
  mtime INTEGER NOT NULL,
  pagerank REAL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS symbols(
  id INTEGER PRIMARY KEY,
  file_id INTEGER NOT NULL REFERENCES files(id),
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  line INTEGER NOT NULL,
  end_line INTEGER,
  container TEXT
);
CREATE TABLE IF NOT EXISTS deps(
  src_id INTEGER NOT NULL REFERENCES files(id),
  dst_id INTEGER NOT NULL REFERENCES files(id),
  weight REAL NOT NULL DEFAULT 1,
  PRIMARY KEY(src_id, dst_id)
);
CREATE TABLE IF NOT EXISTS cochange(
  a_id INTEGER NOT NULL REFERENCES files(id),
  b_id INTEGER NOT NULL REFERENCES files(id),
  commits INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY(a_id, b_id)
);
CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(name, kind, path);
CREATE INDEX IF NOT EXISTS idx_deps_dst ON deps(dst_id);
CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_id);
CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_cochange_a ON cochange(a_id);
CREATE INDEX IF NOT EXISTS idx_cochange_b ON cochange(b_id);
";

/// Open (or create) the DB at `path`, enabling WAL + busy timeout, wiping on
/// version mismatch or corruption. Returns the connection and whether a full
/// reindex is required.
pub fn ensure_schema(path: &std::path::Path) -> Result<(Connection, bool)> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = match Connection::open(path) {
        Ok(c) => c,
        Err(_) => {
            let _ = std::fs::remove_file(path);
            Connection::open(path)?
        }
    };
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
    let version: i32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap_or(0);
    if version != SCHEMA_VERSION {
        conn.execute_batch(
            "DROP TABLE IF EXISTS files; DROP TABLE IF EXISTS symbols; DROP TABLE IF EXISTS deps; DROP TABLE IF EXISTS cochange; DROP TABLE IF EXISTS symbols_fts;",
        )?;
        conn.execute_batch(DDL)?;
        conn.execute_batch(&format!("PRAGMA user_version={SCHEMA_VERSION};"))?;
        return Ok((conn, true));
    }
    // Validate tables exist (corruption guard); wipe on any failure.
    let valid: Result<String, _> = conn.query_row(
        "SELECT name FROM sqlite_master WHERE name='files';",
        [],
        |r| r.get(0),
    );
    if valid.is_err() {
        drop(conn);
        let _ = std::fs::remove_file(path);
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        conn.execute_batch(DDL)?;
        conn.execute_batch(&format!("PRAGMA user_version={SCHEMA_VERSION};"))?;
        return Ok((conn, true));
    }
    Ok((conn, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_db_needs_reindex() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("codegraph.db");
        let (_conn, needs) = ensure_schema(&db).unwrap();
        assert!(needs);
    }

    #[test]
    fn second_open_is_clean() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("codegraph.db");
        let (conn, _) = ensure_schema(&db).unwrap();
        drop(conn);
        let (_conn2, needs2) = ensure_schema(&db).unwrap();
        assert!(!needs2);
    }
}
