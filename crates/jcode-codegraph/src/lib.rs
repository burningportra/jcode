//! `jcode-codegraph` (`jcode-dj6`): per-repo SQLite code graph — schema, scan,
//! rank, query, symbols.
//!
//! Pure library, no tokio at core (callers wrap in `spawn_blocking`). The
//! index is a cache: `PRAGMA user_version` wipe-and-rebuild on schema change,
//! corrupt DB → delete + rebuild, tools report `source: live` meanwhile.

pub mod query;
pub mod rank;
pub mod scan;
pub mod schema;
pub mod symbols;

pub use query::{CoChange, DepEdge, GraphDb, SymbolRow};
pub use rank::{cochanges_from_log, pagerank};
pub use scan::{ScanExclusions, scan_repo};
pub use schema::{SCHEMA_VERSION, ensure_schema};
pub use symbols::{ExportedSymbol, detect_language, exported_symbols, import_specs};
