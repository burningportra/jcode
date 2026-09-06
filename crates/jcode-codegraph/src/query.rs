//! Indexed queries over the SQLite graph: dependents, dependencies, symbols,
//! co-changes, blast radius. Same output shapes as the live core, plus
//! `source: index`. FTS rows written in the same transaction as symbol rows
//! (plan pass-8 FTS sync contract).

use anyhow::Result;
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct DepEdge {
    pub path: String,
    pub weight: f64,
}

#[derive(Debug, Clone)]
pub struct SymbolRow {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub line: i64,
}

#[derive(Debug, Clone)]
pub struct CoChange {
    pub path: String,
    pub commits: i64,
}

pub struct GraphDb {
    conn: Connection,
}

impl GraphDb {
    pub fn open(path: &Path) -> Result<Self> {
        let (conn, _needs) = crate::schema::ensure_schema(path)?;
        Ok(Self { conn })
    }

    /// Full reindex of `root`: scan, parse, insert files/symbols/deps,
    /// co-changes from git, FTS sync in-transaction, PageRank store.
    /// Returns file count. Increments only what changed when `incremental`.
    pub fn reindex(
        &mut self,
        root: &Path,
        exclusions: &crate::scan::ScanExclusions,
        incremental: bool,
    ) -> Result<usize> {
        let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let files = crate::scan::scan_repo(&canon_root, exclusions);
        // Changed-file detection for incremental: compare mtimes.
        let known: HashMap<String, i64> = self
            .conn
            .prepare("SELECT path, mtime FROM files")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        let tx = self.conn.transaction()?;
        let mut file_ids: HashMap<String, i64> = HashMap::new();
        let mut count = 0;
        for f in &files {
            let changed = !incremental || known.get(&f.rel).map(|m| *m != f.mtime).unwrap_or(true);
            // Upsert file row always (mtime may have changed).
            tx.execute(
                "INSERT INTO files(path, language, mtime) VALUES(?1,?2,?3)
                 ON CONFLICT(path) DO UPDATE SET language=excluded.language, mtime=excluded.mtime",
                params![f.rel, f.language, f.mtime],
            )?;
            let id: i64 =
                tx.query_row("SELECT id FROM files WHERE path=?1", params![f.rel], |r| {
                    r.get(0)
                })?;
            file_ids.insert(f.rel.clone(), id);
            if !changed {
                continue;
            }
            count += 1;
            // Re-parse changed file: delete + reinsert symbols/deps/FTS.
            tx.execute("DELETE FROM symbols WHERE file_id=?1", params![id])?;
            tx.execute("DELETE FROM deps WHERE src_id=?1", params![id])?;
            tx.execute("DELETE FROM symbols_fts WHERE path=?1", params![f.rel])?;
            let content = std::fs::read_to_string(&f.abs).unwrap_or_default();
            for s in crate::symbols::exported_symbols(&f.rel, &content) {
                tx.execute(
                    "INSERT INTO symbols(file_id, name, kind, line, end_line) VALUES(?1,?2,?3,?4,?5)",
                    params![id, s.name, s.kind, s.line as i64, s.end_line.map(|e| e as i64)],
                )?;
                tx.execute(
                    "INSERT INTO symbols_fts(name, kind, path) VALUES(?1,?2,?3)",
                    params![s.name, s.kind, f.rel],
                )?;
            }
            // Deps: resolve imports; skip self-deps (plan pass-2).
            let mut seen = std::collections::HashSet::new();
            for spec in crate::symbols::import_specs(&f.rel, &content) {
                if let Some(dst_rel) = crate::scan::resolve_import(&canon_root, &f.rel, &spec) {
                    if dst_rel == f.rel || !seen.insert(dst_rel.clone()) {
                        continue;
                    }
                    // Ensure dst file row exists (may be unscanned ext — insert stub).
                    tx.execute(
                        "INSERT OR IGNORE INTO files(path, language, mtime) VALUES(?1,'unknown',0)",
                        params![dst_rel],
                    )?;
                    let dst_id: i64 = tx.query_row(
                        "SELECT id FROM files WHERE path=?1",
                        params![dst_rel],
                        |r| r.get(0),
                    )?;
                    if dst_id == id {
                        continue;
                    }
                    tx.execute(
                        "INSERT OR IGNORE INTO deps(src_id, dst_id, weight) VALUES(?1,?2,1)",
                        params![id, dst_id],
                    )?;
                }
            }
        }
        tx.commit()?;
        // Co-changes + PageRank outside the file tx (separate concerns).
        self.store_cochanges(&canon_root)?;
        self.store_pagerank()?;
        Ok(count)
    }

    fn store_cochanges(&mut self, root: &Path) -> Result<()> {
        let out = std::process::Command::new("git")
            .args([
                "-C",
                &root.to_string_lossy().to_string(),
                "log",
                "--name-only",
                "--pretty=format:COMMIT:%H",
                "--max-count=500",
            ])
            .output();
        let Ok(out) = out else { return Ok(()) };
        if !out.status.success() {
            return Ok(());
        }
        let pairs = crate::rank::cochanges_from_log(&String::from_utf8_lossy(&out.stdout));
        // Map paths to ids (only tracked files).
        let ids: HashMap<String, i64> = self
            .conn
            .prepare("SELECT path, id FROM files")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM cochange", [])?;
        for ((a, b), c) in pairs {
            if let (Some(aid), Some(bid)) = (ids.get(&a), ids.get(&b)) {
                let (x, y) = if aid < bid { (aid, bid) } else { (bid, aid) };
                tx.execute(
                    "INSERT INTO cochange(a_id, b_id, commits) VALUES(?1,?2,?3)",
                    params![x, y, c as i64],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn store_pagerank(&mut self) -> Result<()> {
        let nodes: Vec<String> = self
            .conn
            .prepare("SELECT path FROM files")?
            .query_map([], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        let id_of: HashMap<String, i64> = self
            .conn
            .prepare("SELECT path, id FROM files")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        let path_of: HashMap<i64, String> = id_of.iter().map(|(k, v)| (*v, k.clone())).collect();
        let edges: Vec<(i64, i64, f64)> = self
            .conn
            .prepare("SELECT src_id, dst_id, weight FROM deps")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .filter_map(|r| r.ok())
            .collect();
        // Rank over paths.
        let pedges: Vec<(String, String, f64)> = edges
            .iter()
            .filter_map(|(s, d, w)| Some((path_of.get(s)?.clone(), path_of.get(d)?.clone(), *w)))
            .collect();
        let ranks = crate::rank::pagerank(&nodes, &pedges);
        for (path, rank) in ranks {
            self.conn.execute(
                "UPDATE files SET pagerank=?1 WHERE path=?2",
                params![rank, path],
            )?;
        }
        Ok(())
    }

    fn file_id(&self, rel: &str) -> Result<Option<i64>> {
        let mut stmt = self.conn.prepare("SELECT id FROM files WHERE path=?1")?;
        let mut rows = stmt.query(params![rel])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    pub fn dependents(&self, rel: &str) -> Result<Vec<DepEdge>> {
        let Some(id) = self.file_id(rel)? else {
            return Ok(vec![]);
        };
        let mut stmt = self.conn.prepare(
            "SELECT f.path, d.weight FROM deps d JOIN files f ON f.id=d.src_id WHERE d.dst_id=?1 ORDER BY d.weight DESC LIMIT 100",
        )?;
        Ok(stmt
            .query_map(params![id], |r| {
                Ok(DepEdge {
                    path: r.get(0)?,
                    weight: r.get(1)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect())
    }

    pub fn dependencies(&self, rel: &str) -> Result<Vec<DepEdge>> {
        let Some(id) = self.file_id(rel)? else {
            return Ok(vec![]);
        };
        let mut stmt = self.conn.prepare(
            "SELECT f.path, d.weight FROM deps d JOIN files f ON f.id=d.dst_id WHERE d.src_id=?1 ORDER BY d.weight DESC LIMIT 100",
        )?;
        Ok(stmt
            .query_map(params![id], |r| {
                Ok(DepEdge {
                    path: r.get(0)?,
                    weight: r.get(1)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect())
    }

    pub fn cochanges(&self, rel: &str) -> Result<Vec<CoChange>> {
        let Some(id) = self.file_id(rel)? else {
            return Ok(vec![]);
        };
        let mut stmt = self.conn.prepare(
            "SELECT f.path, c.commits FROM cochange c JOIN files f ON f.id = CASE WHEN c.a_id=?1 THEN c.b_id ELSE c.a_id END WHERE c.a_id=?1 OR c.b_id=?1 ORDER BY c.commits DESC LIMIT 20",
        )?;
        Ok(stmt
            .query_map(params![id], |r| {
                Ok(CoChange {
                    path: r.get(0)?,
                    commits: r.get(1)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect())
    }

    pub fn symbols(&self, rel: &str) -> Result<Vec<SymbolRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.path, s.name, s.kind, s.line FROM symbols s JOIN files f ON f.id=s.file_id WHERE f.path=?1",
        )?;
        Ok(stmt
            .query_map(params![rel], |r| {
                Ok(SymbolRow {
                    path: r.get(0)?,
                    name: r.get(1)?,
                    kind: r.get(2)?,
                    line: r.get(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect())
    }

    /// FTS5 symbol search (parameterized MATCH — never concatenated).
    pub fn find_symbols(&self, query: &str) -> Result<Vec<SymbolRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, name, kind, 0 FROM symbols_fts WHERE symbols_fts MATCH ?1 LIMIT 50",
        )?;
        Ok(stmt
            .query_map(params![query], |r| {
                Ok(SymbolRow {
                    path: r.get(0)?,
                    name: r.get(1)?,
                    kind: r.get(2)?,
                    line: r.get(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("core.ts"), "export function core() {}\n").unwrap();
        std::fs::write(
            d.path().join("user.ts"),
            "import { core } from './core';\ncore();\n",
        )
        .unwrap();
        d
    }

    #[test]
    fn index_and_query_roundtrip() {
        let d = repo();
        let db_path = d.path().join(".jcode").join("codegraph.db");
        let mut db = GraphDb::open(&db_path).unwrap();
        let n = db
            .reindex(d.path(), &crate::scan::ScanExclusions::default(), false)
            .unwrap();
        assert!(n >= 2, "n={n}");
        let dents = db.dependents("core.ts").unwrap();
        assert!(dents.iter().any(|e| e.path == "user.ts"), "dents={dents:?}");
        let deps = db.dependencies("user.ts").unwrap();
        assert!(deps.iter().any(|e| e.path == "core.ts"), "deps={deps:?}");
        let syms = db.symbols("core.ts").unwrap();
        assert!(syms.iter().any(|s| s.name == "core"), "syms={syms:?}");
        let found = db.find_symbols("core").unwrap();
        assert!(found.iter().any(|s| s.name == "core"), "found={found:?}");
    }
}
