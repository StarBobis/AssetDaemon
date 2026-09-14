/*
 * sqlite_store.rs - SQLite-backed AssetMap storage
 *
 * Replaces the JSON file shard system (file_index.rs / index_lookup.rs)
 * with a single SQLite database. WAL-mode enables concurrent reads during
 * build while indexed queries replace per-shard JSON deserialization.
 *
 * Schema: 7 tables - bundles, assets, relations, asset_containers,
 *          asset_externals, asset_internal_names, build_meta.
 * Indexes are created after bulk inserts to maximize build throughput.
 */

use crate::common::asset_map::asset_index::{
    AssetDisplayRow, AssetRow, BundleInfoRow, ContainerRow, ExternalWriteRow, MapBundleWriteRows,
    RelationRow, TextureCandidateRow,
};
use crate::common::asset_map::asset_map_types::{AssetSearchIndexStatus, MapAssetQueryOptions};
use crate::common::task::task_context::TaskContext;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::OpenFlags;
use rusqlite::OptionalExtension;
use rusqlite::Transaction;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

// ============================================================
// DDL constants - table creation without indexes for fast build
// ============================================================

const DDL_BUNDLES: &str = "
    CREATE TABLE IF NOT EXISTS bundles (
        id              INTEGER PRIMARY KEY,
        bundle_path     TEXT    NOT NULL UNIQUE,
        file_name       TEXT    NOT NULL,
        file_size       INTEGER NOT NULL,
        modified_ms     INTEGER NOT NULL,
        unity_version   TEXT    NOT NULL DEFAULT '',
        md5             TEXT    NOT NULL DEFAULT ''
    );
";

const DDL_ASSETS: &str = "
    CREATE TABLE IF NOT EXISTS assets (
        bundle_id   INTEGER NOT NULL REFERENCES bundles(id),
        path_id     INTEGER NOT NULL,
        class_id    INTEGER NOT NULL,
        class_name  TEXT    NOT NULL,
        asset_name  TEXT    NOT NULL DEFAULT '',
        asset_path  TEXT    NOT NULL DEFAULT '',
        byte_size   INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (bundle_id, path_id)
    );
";

const DDL_RELATIONS: &str = "
    CREATE TABLE IF NOT EXISTS relations (
        id                  INTEGER PRIMARY KEY,
        bundle_id           INTEGER NOT NULL REFERENCES bundles(id),
        source_path_id      INTEGER NOT NULL,
        target_path_id      INTEGER NOT NULL,
        relation_type       TEXT    NOT NULL,
        target_name         TEXT    NOT NULL DEFAULT '',
        file_id             INTEGER NOT NULL DEFAULT 0,
        field_path          TEXT    NOT NULL DEFAULT '',
        target_bundle_path  TEXT    NOT NULL DEFAULT ''
    );
";

const DDL_CONTAINERS: &str = "
    CREATE TABLE IF NOT EXISTS asset_containers (
        bundle_id   INTEGER NOT NULL REFERENCES bundles(id),
        path_id     INTEGER NOT NULL,
        asset_path  TEXT    NOT NULL
    );
";

const DDL_EXTERNALS: &str = "
    CREATE TABLE IF NOT EXISTS asset_externals (
        bundle_id   INTEGER NOT NULL REFERENCES bundles(id),
        sf_index    INTEGER NOT NULL DEFAULT 0,
        file_id     INTEGER NOT NULL DEFAULT 0,
        path_name   TEXT    NOT NULL
    );
";

const DDL_INTERNAL_NAMES: &str = "
    CREATE TABLE IF NOT EXISTS asset_internal_names (
        bundle_id   INTEGER NOT NULL REFERENCES bundles(id),
        path_id     INTEGER NOT NULL,
        name        TEXT    NOT NULL,
        kind        TEXT    NOT NULL DEFAULT ''
    );
";

const DDL_BUILD_META: &str = "
    CREATE TABLE IF NOT EXISTS build_meta (
        key     TEXT PRIMARY KEY,
        value   TEXT NOT NULL
    );
";

const DDL_ASSET_SEARCH_FTS: &str = "
    CREATE VIRTUAL TABLE IF NOT EXISTS asset_search_trigram_fts USING fts5(
        asset_name,
        asset_path,
        class_name,
        bundle_path,
        asset_rowid UNINDEXED,
        tokenize = 'trigram'
    );
";

// ============================================================
// Index DDL - created AFTER bulk inserts for maximum throughput
// ============================================================

const BUILD_INDEXES_SQL: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_assets_class ON assets(class_name);",
    "CREATE INDEX IF NOT EXISTS idx_assets_path_id ON assets(path_id);",
    "CREATE INDEX IF NOT EXISTS idx_assets_bundle_class ON assets(bundle_id, class_name);",
    "CREATE INDEX IF NOT EXISTS idx_rel_source ON relations(bundle_id, relation_type, source_path_id);",
    "CREATE INDEX IF NOT EXISTS idx_rel_target ON relations(target_bundle_path, target_path_id);",
    "CREATE INDEX IF NOT EXISTS idx_containers_bundle ON asset_containers(bundle_id);",
    "CREATE INDEX IF NOT EXISTS idx_externals_bundle ON asset_externals(bundle_id);",
    "CREATE INDEX IF NOT EXISTS idx_externals_path ON asset_externals(path_name);",
    "CREATE INDEX IF NOT EXISTS idx_internal_name ON asset_internal_names(name);",
];

const DROP_INDEXES_SQL: &[&str] = &[
    "DROP INDEX IF EXISTS idx_assets_class;",
    "DROP INDEX IF EXISTS idx_assets_path_id;",
    "DROP INDEX IF EXISTS idx_assets_bundle_class;",
    "DROP INDEX IF EXISTS idx_rel_source;",
    "DROP INDEX IF EXISTS idx_rel_target;",
    "DROP INDEX IF EXISTS idx_containers_bundle;",
    "DROP INDEX IF EXISTS idx_externals_bundle;",
    "DROP INDEX IF EXISTS idx_externals_path;",
    "DROP INDEX IF EXISTS idx_internal_name;",
];

const ASSET_SEARCH_META_KEY: &str = "asset_search_trigram_fts_asset_count";
const ASSET_SEARCH_REBUILD_CHUNK_SIZE: i64 = 20_000;

// ============================================================
// Build-time bulk insert size threshold
// Flush to disk every N bundles to balance memory vs. disk I/O
// ============================================================

const BUNDLE_FLUSH_THRESHOLD: usize = 256;

// ============================================================
// SqliteStore - the single replacement for FileIndexBuildSession
//               and all JSON shard read paths
// ============================================================

pub struct SqliteStore {
    db_path: PathBuf,
}

pub struct SqliteBuildSession {
    #[allow(dead_code)]
    store: SqliteStore,
    conn: Connection,
    pending_bundles: Vec<Arc<MapBundleWriteRows>>,
    asset_count: usize,
    bundle_count: usize,
    parsed_count: usize,
}

struct BuildInsertStatements<'conn> {
    insert_bundle: rusqlite::Statement<'conn>,
    insert_asset: rusqlite::Statement<'conn>,
    insert_container: rusqlite::Statement<'conn>,
    insert_external: rusqlite::Statement<'conn>,
    insert_internal_name: rusqlite::Statement<'conn>,
    insert_relation: rusqlite::Statement<'conn>,
}

impl SqliteStore {
    // ----------------------------------------------------------
    // Open / Create
    // ----------------------------------------------------------

    pub fn open(workspace: &Path, cache_root: Option<&Path>) -> Result<Self, String> {
        let db_path = Self::db_path(workspace, cache_root);
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create db dir: {}", e))?;
        }
        Ok(Self { db_path })
    }

    pub fn exists(workspace: &Path, cache_root: Option<&Path>) -> bool {
        let db_path = Self::db_path(workspace, cache_root);
        db_path.exists() && Self::has_required_schema(&db_path)
    }

    fn db_path(workspace: &Path, cache_root: Option<&Path>) -> PathBuf {
        match cache_root {
            Some(root) => {
                let dir = crate::common::asset_map::asset_index::AssetDatabase::workspace_cache_dir(
                    root, workspace,
                );
                dir.join("asset_index.db")
            }
            None => workspace.join("asset_index.db"),
        }
    }

    fn has_required_schema(db_path: &Path) -> bool {
        let conn = match Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(conn) => conn,
            Err(_) => return false,
        };
        [
            "bundles",
            "assets",
            "relations",
            "asset_containers",
            "asset_externals",
            "asset_internal_names",
            "build_meta",
        ]
        .iter()
        .all(|table| {
            conn.query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
                params![table],
                |_| Ok(()),
            )
            .optional()
            .ok()
            .flatten()
            .is_some()
        })
    }

    fn open_connection(&self) -> Result<Connection, String> {
        let conn = Connection::open(&self.db_path)
            .map_err(|e| format!("open db {}: {}", self.db_path.display(), e))?;
        let _: String = conn
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .map_err(|e| format!("enable WAL: {}", e))?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")
            .map_err(|e| format!("set synchronous: {}", e))?;
        conn.execute_batch("PRAGMA cache_size=-64000;")
            .map_err(|e| format!("set cache_size: {}", e))?;
        Ok(conn)
    }

    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
        let conn = self.open_connection()?;
        f(&conn)
    }

    // ----------------------------------------------------------
    // Schema management
    // ----------------------------------------------------------

    pub fn create_tables(&self) -> Result<(), String> {
        self.with_conn(|conn| {
            conn.execute_batch(DDL_BUNDLES)
                .map_err(|e| format!("bundles: {}", e))?;
            conn.execute_batch(DDL_ASSETS)
                .map_err(|e| format!("assets: {}", e))?;
            conn.execute_batch(DDL_RELATIONS)
                .map_err(|e| format!("relations: {}", e))?;
            conn.execute_batch(DDL_CONTAINERS)
                .map_err(|e| format!("containers: {}", e))?;
            conn.execute_batch(DDL_EXTERNALS)
                .map_err(|e| format!("externals: {}", e))?;
            conn.execute_batch(DDL_INTERNAL_NAMES)
                .map_err(|e| format!("internal_names: {}", e))?;
            conn.execute_batch(DDL_BUILD_META)
                .map_err(|e| format!("build_meta: {}", e))?;
            Self::create_asset_search_table(conn)?;
            Ok(())
        })
    }

    pub fn drop_indexes(&self) -> Result<(), String> {
        self.with_conn(|conn| {
            for sql in DROP_INDEXES_SQL {
                conn.execute(sql, [])
                    .map_err(|e| format!("drop index: {}", e))?;
            }
            Ok(())
        })
    }

    pub fn create_indexes(&self) -> Result<(), String> {
        self.with_conn(|conn| {
            for sql in BUILD_INDEXES_SQL {
                conn.execute(sql, [])
                    .map_err(|e| format!("create index: {}", e))?;
            }
            Self::create_asset_search_table(conn)?;
            conn.execute_batch("PRAGMA optimize;")
                .map_err(|e| format!("optimize: {}", e))?;
            Ok(())
        })
    }

    fn create_asset_search_table(conn: &Connection) -> Result<(), String> {
        conn.execute_batch(DDL_ASSET_SEARCH_FTS)
            .map_err(|e| format!("asset_search_trigram_fts: {}", e))
    }

    fn asset_search_index_status_conn(conn: &Connection) -> Result<AssetSearchIndexStatus, String> {
        Self::create_asset_search_table(conn)?;
        let asset_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM assets", [], |row| row.get(0))
            .unwrap_or(0);
        let indexed_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM asset_search_trigram_fts", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        let indexed_asset_count = conn
            .query_row(
                "SELECT value FROM build_meta WHERE key = ?1",
                params![ASSET_SEARCH_META_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| *value >= 0);
        let ready = asset_count > 0
            && indexed_count == asset_count
            && indexed_asset_count == Some(asset_count);
        Ok(AssetSearchIndexStatus {
            ready,
            asset_count: asset_count.max(0) as usize,
            indexed_count: indexed_count.max(0) as usize,
            indexed_asset_count: indexed_asset_count.map(|value| value as usize),
        })
    }

    fn is_asset_search_index_ready(conn: &Connection) -> bool {
        Self::asset_search_index_status_conn(conn)
            .map(|status| status.ready)
            .unwrap_or(false)
    }

    fn rebuild_asset_search_index(
        conn: &Connection,
        task_ctx: Option<&TaskContext>,
    ) -> Result<AssetSearchIndexStatus, String> {
        Self::create_asset_search_table(conn)?;
        let started = Instant::now();
        let asset_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM assets", [], |row| row.get(0))
            .map_err(|e| format!("count assets for asset_search_trigram_fts: {}", e))?;
        let total = asset_count.max(0) as usize;
        if let Some(ctx) = task_ctx {
            ctx.info(format!(
                "Building All Assets trigram search index for {} assets",
                total
            ));
            ctx.progress("Search Index", 0, total.max(1), "Clearing old search index");
        }

        conn.execute_batch("BEGIN IMMEDIATE;")
            .map_err(|e| format!("begin asset_search_trigram_fts rebuild: {}", e))?;
        let rebuild_result = (|| -> Result<usize, String> {
            conn.execute("DELETE FROM asset_search_trigram_fts", [])
                .map_err(|e| format!("clear asset_search_trigram_fts: {}", e))?;

            let max_rowid: i64 = conn
                .query_row("SELECT COALESCE(MAX(rowid), 0) FROM assets", [], |row| {
                    row.get(0)
                })
                .map_err(|e| format!("asset_search_trigram_fts max rowid: {}", e))?;
            let mut inserted = 0usize;
            let mut start_rowid = 1i64;
            while start_rowid <= max_rowid {
                if let Some(ctx) = task_ctx {
                    ctx.check_cancelled()?;
                }
                let end_rowid = (start_rowid + ASSET_SEARCH_REBUILD_CHUNK_SIZE - 1).min(max_rowid);
                let changed = conn
                    .execute(
                        "INSERT INTO asset_search_trigram_fts (asset_name, asset_path, class_name, bundle_path, asset_rowid)
                         SELECT a.asset_name, a.asset_path, a.class_name, b.bundle_path, a.rowid
                         FROM assets a
                         JOIN bundles b ON b.id = a.bundle_id
                         WHERE a.rowid BETWEEN ?1 AND ?2",
                        params![start_rowid, end_rowid],
                    )
                    .map_err(|e| format!("populate asset_search_trigram_fts: {}", e))?;
                inserted = inserted.saturating_add(changed);
                if let Some(ctx) = task_ctx {
                    let elapsed = started.elapsed().as_secs();
                    ctx.progress(
                        "Search Index",
                        inserted.min(total),
                        total.max(1),
                        format!(
                            "Indexed {}/{} assets, elapsed {}s",
                            inserted.min(total),
                            total,
                            elapsed
                        ),
                    );
                }
                start_rowid = end_rowid + 1;
            }

            conn.execute(
                "INSERT OR REPLACE INTO build_meta (key, value) VALUES (?1, ?2)",
                params![ASSET_SEARCH_META_KEY, asset_count.to_string()],
            )
            .map_err(|e| format!("asset_search_trigram_fts meta: {}", e))?;
            Ok(inserted)
        })();
        match rebuild_result {
            Ok(_) => conn
                .execute_batch("COMMIT;")
                .map_err(|e| format!("commit asset_search_trigram_fts rebuild: {}", e))?,
            Err(error) => {
                conn.execute_batch("ROLLBACK;").ok();
                return Err(error);
            }
        }

        let status = Self::asset_search_index_status_conn(conn)?;
        if let Some(ctx) = task_ctx {
            ctx.success(format!(
                "All Assets trigram search index ready: {}/{} assets, took {}s",
                status.indexed_count,
                status.asset_count,
                started.elapsed().as_secs()
            ));
        }
        Ok(status)
    }

    fn index_bundle_assets_for_search(conn: &Connection, bundle_id: i64) -> Result<(), String> {
        Self::create_asset_search_table(conn)?;
        conn.execute(
            "DELETE FROM asset_search_trigram_fts
             WHERE asset_rowid IN (SELECT rowid FROM assets WHERE bundle_id = ?1)",
            params![bundle_id],
        )
        .ok();
        conn.execute(
            "INSERT INTO asset_search_trigram_fts (asset_name, asset_path, class_name, bundle_path, asset_rowid)
             SELECT a.asset_name, a.asset_path, a.class_name, b.bundle_path, a.rowid
             FROM assets a
             JOIN bundles b ON b.id = a.bundle_id
             WHERE a.bundle_id = ?1",
            params![bundle_id],
        )
        .map_err(|e| format!("index bundle assets for search: {}", e))?;
        conn.execute(
            "DELETE FROM build_meta WHERE key = ?1",
            params![ASSET_SEARCH_META_KEY],
        )
        .ok();
        Ok(())
    }

    pub fn asset_search_index_status(&self) -> Result<AssetSearchIndexStatus, String> {
        self.with_conn(Self::asset_search_index_status_conn)
    }

    pub fn build_asset_search_index(
        &self,
        task_ctx: Option<&TaskContext>,
    ) -> Result<AssetSearchIndexStatus, String> {
        self.with_conn(|conn| Self::rebuild_asset_search_index(conn, task_ctx))
    }

    // ----------------------------------------------------------
    // Build session lifecycle
    // ----------------------------------------------------------

    pub fn begin_build_session(&self) -> Result<SqliteBuildSession, String> {
        Self::delete_db_path(&self.db_path)?;
        let conn = self.open_connection()?;

        // WAL journal mode for concurrent access during build
        let _: String = conn
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .map_err(|e| format!("WAL: {}", e))?;
        conn.execute_batch("PRAGMA synchronous=OFF;")
            .map_err(|e| format!("sync: {}", e))?;

        conn.execute_batch(DDL_BUNDLES).ok();
        conn.execute_batch(DDL_ASSETS).ok();
        conn.execute_batch(DDL_RELATIONS).ok();
        conn.execute_batch(DDL_CONTAINERS).ok();
        conn.execute_batch(DDL_EXTERNALS).ok();
        conn.execute_batch(DDL_INTERNAL_NAMES).ok();
        conn.execute_batch(DDL_BUILD_META).ok();
        Self::create_asset_search_table(&conn).ok();

        Ok(SqliteBuildSession {
            store: self.clone(),
            conn,
            pending_bundles: Vec::with_capacity(BUNDLE_FLUSH_THRESHOLD),
            asset_count: 0,
            bundle_count: 0,
            parsed_count: 0,
        })
    }

    // ----------------------------------------------------------
    // Build session -- caller drives append + flush + finish
    // ----------------------------------------------------------

    fn append_bundle_rows(
        tx: &Transaction<'_>,
        statements: &mut BuildInsertStatements<'_>,
        rows: &MapBundleWriteRows,
        asset_count: &mut usize,
        bundle_count: &mut usize,
        parsed_count: &mut usize,
    ) -> Result<(), String> {
        statements
            .insert_bundle
            .execute(params![
                rows.bundle_path,
                Path::new(&rows.bundle_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(""),
                rows.file_size as i64,
                rows.modified_ms as i64,
                rows.unity_version,
                rows.md5,
            ])
            .map_err(|e| format!("insert bundle {}: {}", rows.bundle_path, e))?;

        let bundle_id: i64 = tx
            .query_row(
                "SELECT id FROM bundles WHERE bundle_path = ?1",
                params![rows.bundle_path],
                |row| row.get(0),
            )
            .map_err(|e| format!("query bundle id {}: {}", rows.bundle_path, e))?;
        let container_path_by_id = rows
            .containers
            .iter()
            .map(|row| (row.path_id, row.asset_path.as_str()))
            .collect::<std::collections::HashMap<_, _>>();

        for asset in &rows.assets {
            statements
                .insert_asset
                .execute(params![
                    bundle_id,
                    asset.path_id,
                    asset.class_id,
                    asset.class_name,
                    asset.asset_name,
                    container_path_by_id
                        .get(&asset.path_id)
                        .copied()
                        .unwrap_or(""),
                    asset.byte_size,
                ])
                .map_err(|e| {
                    format!("insert asset {}:{}: {}", rows.bundle_path, asset.path_id, e)
                })?;
            *asset_count += 1;
        }

        for c in &rows.containers {
            statements
                .insert_container
                .execute(params![bundle_id, c.path_id, c.asset_path])
                .map_err(|e| {
                    format!("insert container {}:{}: {}", rows.bundle_path, c.path_id, e)
                })?;
        }

        for ext in &rows.externals {
            statements
                .insert_external
                .execute(params![
                    bundle_id,
                    ext.sf_index as i64,
                    ext.file_id,
                    ext.path_name
                ])
                .map_err(|e| {
                    format!(
                        "insert external {}:{}: {}",
                        rows.bundle_path, ext.path_name, e
                    )
                })?;
        }

        for n in &rows.internal_names {
            statements
                .insert_internal_name
                .execute(params![bundle_id, 0_i64, n.name, n.kind])
                .map_err(|e| {
                    format!(
                        "insert internal_name {}:{}: {}",
                        rows.bundle_path, n.name, e
                    )
                })?;
        }

        for relation in &rows.relations {
            statements
                .insert_relation
                .execute(params![
                    bundle_id,
                    relation.source_path_id,
                    relation.target_path_id,
                    relation.relation_type,
                    relation.target_name,
                    relation.file_id,
                    relation.field_path,
                    relation.target_bundle_path,
                ])
                .map_err(|e| {
                    format!(
                        "insert relation {}:{} -> {} ({}): {}",
                        rows.bundle_path,
                        relation.source_path_id,
                        relation.target_path_id,
                        relation.relation_type,
                        e
                    )
                })?;
        }

        *bundle_count += 1;
        *parsed_count += 1;

        Ok(())
    }

    fn prepare_build_statements<'conn>(
        tx: &'conn Transaction<'conn>,
    ) -> Result<BuildInsertStatements<'conn>, String> {
        Ok(BuildInsertStatements {
            insert_bundle: tx
                .prepare(
                    "INSERT INTO bundles (bundle_path, file_name, file_size, modified_ms, unity_version, md5)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(|e| format!("prepare bundle insert: {}", e))?,
            insert_asset: tx
                .prepare(
                    "INSERT OR REPLACE INTO assets
                     (bundle_id, path_id, class_id, class_name, asset_name, asset_path, byte_size)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )
                .map_err(|e| format!("prepare asset insert: {}", e))?,
            insert_container: tx
                .prepare(
                    "INSERT INTO asset_containers (bundle_id, path_id, asset_path)
                     VALUES (?1, ?2, ?3)",
                )
                .map_err(|e| format!("prepare container insert: {}", e))?,
            insert_external: tx
                .prepare(
                    "INSERT INTO asset_externals (bundle_id, sf_index, file_id, path_name)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(|e| format!("prepare external insert: {}", e))?,
            insert_internal_name: tx
                .prepare(
                    "INSERT INTO asset_internal_names (bundle_id, path_id, name, kind)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(|e| format!("prepare internal_name insert: {}", e))?,
            insert_relation: tx
                .prepare(
                    "INSERT INTO relations
                     (bundle_id, source_path_id, target_path_id, relation_type,
                      target_name, file_id, field_path, target_bundle_path)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )
                .map_err(|e| format!("prepare relation insert: {}", e))?,
        })
    }

    // ----------------------------------------------------------
    // Clone helper (SqliteStore is cheap to clone - just path)
    // ----------------------------------------------------------

    fn clone(&self) -> Self {
        Self {
            db_path: self.db_path.clone(),
        }
    }
}

impl SqliteBuildSession {
    /// Append parsed bundle rows to the in-memory pending queue.
    /// Flushes to disk when pending reaches BUNDLE_FLUSH_THRESHOLD.
    pub fn append_bundle(&mut self, rows: Arc<MapBundleWriteRows>) -> Result<usize, String> {
        let asset_len = rows.assets.len();
        self.pending_bundles.push(rows);
        if self.pending_bundles.len() >= BUNDLE_FLUSH_THRESHOLD {
            self.flush()?;
        }
        Ok(asset_len)
    }

    /// Append a pre-built shard (from cache hit). Converts to write rows and inserts.
    #[allow(dead_code)]
    pub fn append_l1_shard(
        &mut self,
        shard: &crate::common::asset_map::file_index::FileIndexBundleShard,
    ) -> Result<usize, String> {
        let asset_count = shard.assets.len();
        let write_rows = MapBundleWriteRows {
            bundle_path: shard.bundle_path.clone(),
            md5: shard.md5.clone(),
            file_size: shard.file_size,
            modified_ms: shard.modified_ms,
            unity_version: shard.unity_version.clone(),
            asset_count: shard.asset_count,
            assets: shard
                .assets
                .iter()
                .map(|a| crate::common::asset_map::asset_index::AssetWriteRow {
                    bundle_path: a.bundle_path.clone(),
                    path_id: a.path_id,
                    class_id: a.class_id,
                    class_name: a.class_name.clone(),
                    asset_name: a.asset_name.clone(),
                    byte_size: a.byte_size,
                })
                .collect(),
            containers: shard.containers.clone(),
            externals: shard.externals.clone(),
            internal_names: shard.internal_names.clone(),
            relations: shard.relations.clone(),
        };
        self.pending_bundles.push(Arc::new(write_rows));
        if self.pending_bundles.len() >= BUNDLE_FLUSH_THRESHOLD {
            self.flush()?;
        }
        Ok(asset_count)
    }

    pub fn flush(&mut self) -> Result<(), String> {
        if self.pending_bundles.is_empty() {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("begin tx: {}", e))?;
        {
            let mut statements = SqliteStore::prepare_build_statements(&tx)?;

            for rows in &self.pending_bundles {
                SqliteStore::append_bundle_rows(
                    &tx,
                    &mut statements,
                    rows,
                    &mut self.asset_count,
                    &mut self.bundle_count,
                    &mut self.parsed_count,
                )?;
            }
        }

        tx.commit().map_err(|e| format!("commit flush: {}", e))?;

        self.pending_bundles.clear();
        Ok(())
    }

    pub fn finish(
        mut self,
        built_at: u64,
        built_at_formatted: String,
        parsed_count: usize,
        cancelled: bool,
    ) -> Result<crate::common::asset_map::asset_map_types::MapSummary, String> {
        // Flush remaining pending bundles
        self.flush()?;

        // Create indexes now that all data is written
        for sql in BUILD_INDEXES_SQL {
            self.conn
                .execute(sql, [])
                .map_err(|e| format!("create index: {}", e))?;
        }
        SqliteStore::create_asset_search_table(&self.conn)?;
        self.conn
            .execute("DELETE FROM asset_search_trigram_fts", [])
            .map_err(|e| format!("clear asset_search_trigram_fts: {}", e))?;
        self.conn
            .execute(
                "DELETE FROM build_meta WHERE key = ?1",
                params![ASSET_SEARCH_META_KEY],
            )
            .ok();

        // Write build metadata
        self.conn
            .execute(
                "INSERT OR REPLACE INTO build_meta (key, value) VALUES (?1, ?2)",
                params!["built_at", built_at.to_string()],
            )
            .map_err(|e| format!("meta built_at: {}", e))?;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO build_meta (key, value) VALUES (?1, ?2)",
                params!["built_at_formatted", built_at_formatted],
            )
            .map_err(|e| format!("meta built_at_formatted: {}", e))?;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO build_meta (key, value) VALUES (?1, ?2)",
                params!["bundle_count", self.bundle_count.to_string()],
            )
            .map_err(|e| format!("meta bundle_count: {}", e))?;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO build_meta (key, value) VALUES (?1, ?2)",
                params!["asset_count", self.asset_count.to_string()],
            )
            .map_err(|e| format!("meta asset_count: {}", e))?;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO build_meta (key, value) VALUES (?1, ?2)",
                params!["parsed_count", parsed_count.to_string()],
            )
            .map_err(|e| format!("meta parsed_count: {}", e))?;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO build_meta (key, value) VALUES (?1, ?2)",
                params!["cancelled", if cancelled { "1" } else { "0" }],
            )
            .map_err(|e| format!("meta cancelled: {}", e))?;

        self.conn
            .execute_batch("PRAGMA optimize;")
            .map_err(|e| format!("optimize: {}", e))?;
        self.conn
            .execute_batch("PRAGMA synchronous=NORMAL;")
            .map_err(|e| format!("sync normal: {}", e))?;

        Ok(crate::common::asset_map::asset_map_types::MapSummary {
            built_at,
            built_at_formatted,
            bundle_count: self.bundle_count,
            asset_count: self.asset_count,
            parsed_count,
            cancelled,
        })
    }
}

// ============================================================
// Query methods - replacing FileIndexLookup / FileAssetIndex
// ============================================================

impl SqliteStore {
    // ----------------------------------------------------------
    // Build metadata
    // ----------------------------------------------------------

    #[allow(dead_code)]
    pub fn built_at(&self) -> Result<u64, String> {
        self.with_conn(|conn| {
            let result: Result<String, _> = conn.query_row(
                "SELECT value FROM build_meta WHERE key = 'built_at'",
                [],
                |row| row.get(0),
            );
            match result {
                Ok(s) => s.parse().map_err(|e| format!("parse built_at: {}", e)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
                Err(e) => Err(format!("built_at: {}", e)),
            }
        })
    }

    #[allow(dead_code)]
    pub fn build_cancelled(&self) -> Result<bool, String> {
        self.with_conn(|conn| {
            let result: Result<String, _> = conn.query_row(
                "SELECT value FROM build_meta WHERE key = 'cancelled'",
                [],
                |row| row.get(0),
            );
            match result {
                Ok(s) => Ok(s == "1"),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
                Err(e) => Err(format!("cancelled: {}", e)),
            }
        })
    }

    #[allow(dead_code)]
    pub fn bundle_count(&self) -> Result<usize, String> {
        self.with_conn(|conn| {
            conn.query_row("SELECT COUNT(*) FROM bundles", [], |row| {
                row.get::<_, i64>(0).map(|v| v as usize)
            })
            .map_err(|e| format!("bundle_count: {}", e))
        })
    }

    #[allow(dead_code)]
    pub fn asset_count(&self) -> Result<usize, String> {
        self.with_conn(|conn| {
            conn.query_row("SELECT COUNT(*) FROM assets", [], |row| {
                row.get::<_, i64>(0).map(|v| v as usize)
            })
            .map_err(|e| format!("asset_count: {}", e))
        })
    }

    pub fn file_size(&self) -> u64 {
        std::fs::metadata(&self.db_path)
            .map(|m| m.len())
            .unwrap_or(0)
    }

    // ----------------------------------------------------------
    // Bundle queries
    // ----------------------------------------------------------

    pub fn get_bundle_infos(&self) -> Result<Vec<BundleInfoRow>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, b.md5, b.file_size, b.modified_ms,
                            b.unity_version, COUNT(a.bundle_id) as asset_count
                     FROM bundles b
                     LEFT JOIN assets a ON a.bundle_id = b.id
                     GROUP BY b.id
                     ORDER BY b.bundle_path",
                )
                .map_err(|e| format!("get_bundle_infos: {}", e))?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(BundleInfoRow {
                        path: row.get(0)?,
                        md5: row.get(1)?,
                        file_size: row.get::<_, i64>(2)? as u64,
                        modified_ms: row.get::<_, i64>(3)? as u64,
                        unity_version: row.get::<_, String>(4)?,
                        asset_count: row.get::<_, i64>(5)? as usize,
                    })
                })
                .map_err(|e| format!("get_bundle_infos query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("get_bundle_infos row: {}", e))?;

            Ok(rows)
        })
    }

    pub fn get_bundle_info(&self, bundle_path: &str) -> Result<Option<BundleInfoRow>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, b.md5, b.file_size, b.modified_ms,
                            b.unity_version, COUNT(a.bundle_id) as asset_count
                     FROM bundles b
                     LEFT JOIN assets a ON a.bundle_id = b.id
                     WHERE b.bundle_path = ?1
                     GROUP BY b.id",
                )
                .map_err(|e| format!("get_bundle_info: {}", e))?;

            let row = stmt
                .query_row(params![bundle_path], |row| {
                    Ok(BundleInfoRow {
                        path: row.get(0)?,
                        md5: row.get(1)?,
                        file_size: row.get::<_, i64>(2)? as u64,
                        modified_ms: row.get::<_, i64>(3)? as u64,
                        unity_version: row.get::<_, String>(4)?,
                        asset_count: row.get::<_, i64>(5)? as usize,
                    })
                })
                .optional()
                .map_err(|e| format!("get_bundle_info query: {}", e))?;

            Ok(row)
        })
    }

    pub fn find_bundle_paths_by_class(&self, class_name: &str) -> Result<Vec<String>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT b.bundle_path
                     FROM bundles b
                     JOIN assets a ON a.bundle_id = b.id
                     WHERE a.class_name = ?1
                     ORDER BY b.bundle_path",
                )
                .map_err(|e| format!("find_bundle_paths_by_class: {}", e))?;

            let rows = stmt
                .query_map(params![class_name], |row| row.get(0))
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<String>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    // ----------------------------------------------------------
    // Asset queries
    // ----------------------------------------------------------

    pub fn find_by_bundle_and_path_id(
        &self,
        bundle_path: &str,
        path_id: i64,
    ) -> Result<Option<AssetRow>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, a.path_id, a.class_id, a.class_name,
                            a.asset_name, a.byte_size
                     FROM assets a
                     JOIN bundles b ON b.id = a.bundle_id
                     WHERE b.bundle_path = ?1 AND a.path_id = ?2",
                )
                .map_err(|e| format!("find_by_bundle_and_path_id: {}", e))?;

            let row = stmt
                .query_row(params![bundle_path, path_id], |row| {
                    Ok(AssetRow {
                        bundle_path: row.get(0)?,
                        path_id: row.get(1)?,
                        class_id: row.get(2)?,
                        class_name: row.get(3)?,
                        asset_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        byte_size: row.get::<_, i64>(5)? as u32,
                    })
                })
                .optional()
                .map_err(|e| format!("query: {}", e))?;

            Ok(row)
        })
    }

    pub fn find_by_bundle(&self, bundle_path: &str) -> Result<Vec<AssetRow>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, a.path_id, a.class_id, a.class_name,
                            a.asset_name, a.byte_size
                     FROM assets a
                     JOIN bundles b ON b.id = a.bundle_id
                     WHERE b.bundle_path = ?1",
                )
                .map_err(|e| format!("find_by_bundle: {}", e))?;

            let rows = stmt
                .query_map(params![bundle_path], |row| {
                    Ok(AssetRow {
                        bundle_path: row.get(0)?,
                        path_id: row.get(1)?,
                        class_id: row.get(2)?,
                        class_name: row.get(3)?,
                        asset_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        byte_size: row.get::<_, i64>(5)? as u32,
                    })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    pub fn find_by_path_id(&self, path_id: i64) -> Result<Vec<AssetRow>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, a.path_id, a.class_id, a.class_name,
                            a.asset_name, a.byte_size
                     FROM assets a
                     JOIN bundles b ON b.id = a.bundle_id
                     WHERE a.path_id = ?1",
                )
                .map_err(|e| format!("find_by_path_id: {}", e))?;

            let rows = stmt
                .query_map(params![path_id], |row| {
                    Ok(AssetRow {
                        bundle_path: row.get(0)?,
                        path_id: row.get(1)?,
                        class_id: row.get(2)?,
                        class_name: row.get(3)?,
                        asset_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        byte_size: row.get::<_, i64>(5)? as u32,
                    })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    pub fn find_by_class(&self, class_name: &str) -> Result<Vec<AssetRow>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, a.path_id, a.class_id, a.class_name,
                            a.asset_name, a.byte_size
                     FROM assets a
                     JOIN bundles b ON b.id = a.bundle_id
                     WHERE a.class_name = ?1",
                )
                .map_err(|e| format!("find_by_class: {}", e))?;

            let rows = stmt
                .query_map(params![class_name], |row| {
                    Ok(AssetRow {
                        bundle_path: row.get(0)?,
                        path_id: row.get(1)?,
                        class_id: row.get(2)?,
                        class_name: row.get(3)?,
                        asset_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        byte_size: row.get::<_, i64>(5)? as u32,
                    })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    pub fn find_by_class_and_bundle(
        &self,
        class_name: &str,
        bundle_path: &str,
    ) -> Result<Vec<AssetRow>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, a.path_id, a.class_id, a.class_name,
                            a.asset_name, a.byte_size
                     FROM assets a
                     JOIN bundles b ON b.id = a.bundle_id
                     WHERE a.class_name = ?1 AND b.bundle_path = ?2",
                )
                .map_err(|e| format!("find_by_class_and_bundle: {}", e))?;

            let rows = stmt
                .query_map(params![class_name, bundle_path], |row| {
                    Ok(AssetRow {
                        bundle_path: row.get(0)?,
                        path_id: row.get(1)?,
                        class_id: row.get(2)?,
                        class_name: row.get(3)?,
                        asset_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        byte_size: row.get::<_, i64>(5)? as u32,
                    })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    /// Find Mesh assets whose name matches exactly in other bundles (excluding
    /// `exclude_bundle_path`), ordered by bundle file name for determinism.
    pub fn find_mesh_copies_by_name(
        &self,
        mesh_name: &str,
        exclude_bundle_path: &str,
        limit: usize,
    ) -> Result<Vec<AssetDisplayRow>, String> {
        let mesh_name = mesh_name.trim();
        if mesh_name.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, a.path_id, a.class_id, a.class_name,
                            a.asset_name, a.byte_size, a.asset_path
                     FROM assets a
                     JOIN bundles b ON b.id = a.bundle_id
                     WHERE a.class_name = 'Mesh'
                       AND LOWER(a.asset_name) = LOWER(?1)
                       AND b.bundle_path <> ?2
                     ORDER BY b.file_name
                     LIMIT ?3",
                )
                .map_err(|e| format!("find_mesh_copies_by_name: {}", e))?;

            let rows = stmt
                .query_map(params![mesh_name, exclude_bundle_path, limit as i64], |row| {
                    let asset = AssetRow {
                        bundle_path: row.get(0)?,
                        path_id: row.get(1)?,
                        class_id: row.get(2)?,
                        class_name: row.get(3)?,
                        asset_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        byte_size: row.get::<_, i64>(5)? as u32,
                    };
                    let asset_path: String = row.get::<_, Option<String>>(6)?.unwrap_or_default();
                    Ok(AssetDisplayRow { asset, asset_path })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    // ----------------------------------------------------------
    // Container queries
    // ----------------------------------------------------------

    pub fn get_containers(&self, bundle_path: &str) -> Result<Vec<ContainerRow>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, c.asset_path, c.path_id
                     FROM asset_containers c
                     JOIN bundles b ON b.id = c.bundle_id
                     WHERE b.bundle_path = ?1",
                )
                .map_err(|e| format!("get_containers: {}", e))?;

            let rows = stmt
                .query_map(params![bundle_path], |row| {
                    Ok(ContainerRow {
                        bundle_path: row.get(0)?,
                        asset_path: row.get(1)?,
                        path_id: row.get(2)?,
                    })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    // ----------------------------------------------------------
    // External queries
    // ----------------------------------------------------------

    pub fn find_externals_by_bundle(
        &self,
        bundle_path: &str,
    ) -> Result<Vec<ExternalWriteRow>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, e.sf_index, e.file_id, e.path_name
                     FROM asset_externals e
                     JOIN bundles b ON b.id = e.bundle_id
                     WHERE b.bundle_path = ?1",
                )
                .map_err(|e| format!("find_externals_by_bundle: {}", e))?;

            let rows = stmt
                .query_map(params![bundle_path], |row| {
                    Ok(ExternalWriteRow {
                        bundle_path: row.get(0)?,
                        sf_index: row.get(1)?,
                        file_id: row.get(2)?,
                        path_name: row.get(3)?,
                    })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    #[allow(dead_code)]
    pub fn find_bundle_by_external_path_name(
        &self,
        path_name: &str,
    ) -> Result<Option<String>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path
                     FROM asset_externals e
                     JOIN bundles b ON b.id = e.bundle_id
                     WHERE e.path_name LIKE ?1
                     LIMIT 1",
                )
                .map_err(|e| format!("find_bundle_by_external: {}", e))?;

            let row = stmt
                .query_row(params![format!("%{}", path_name)], |row| row.get(0))
                .optional()
                .map_err(|e| format!("query: {}", e))?;

            Ok(row)
        })
    }

    // ----------------------------------------------------------
    // Internal name queries
    // ----------------------------------------------------------

    pub fn find_bundle_by_internal_name(&self, name: &str) -> Result<Option<String>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path
                     FROM asset_internal_names n
                     JOIN bundles b ON b.id = n.bundle_id
                     WHERE n.name = ?1
                     LIMIT 1",
                )
                .map_err(|e| format!("find_bundle_by_internal_name: {}", e))?;
            let normalized = name.to_lowercase();

            let row = stmt
                .query_row(params![normalized], |row| row.get(0))
                .optional()
                .map_err(|e| format!("query: {}", e))?;

            Ok(row)
        })
    }

    // ----------------------------------------------------------
    // Relation queries
    // ----------------------------------------------------------

    pub fn get_relations_by_bundle(&self, bundle_path: &str) -> Result<Vec<RelationRow>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, r.relation_type, r.source_path_id,
                            r.target_path_id, r.target_name, r.file_id,
                            r.field_path, r.target_bundle_path
                     FROM relations r
                     JOIN bundles b ON b.id = r.bundle_id
                     WHERE b.bundle_path = ?1",
                )
                .map_err(|e| format!("get_relations_by_bundle: {}", e))?;

            let rows = stmt
                .query_map(params![bundle_path], |row| {
                    Ok(RelationRow {
                        bundle_path: row.get(0)?,
                        relation_type: row.get(1)?,
                        source_path_id: row.get(2)?,
                        target_path_id: row.get(3)?,
                        source_name: String::new(),
                        target_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        file_id: row.get(5)?,
                        field_path: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                        target_bundle_path: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
                    })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    pub fn get_relations_by_type(
        &self,
        bundle_path: &str,
        relation_type: &str,
    ) -> Result<Vec<RelationRow>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, r.relation_type, r.source_path_id,
                            r.target_path_id, r.target_name, r.file_id,
                            r.field_path, r.target_bundle_path
                     FROM relations r
                     JOIN bundles b ON b.id = r.bundle_id
                     WHERE b.bundle_path = ?1 AND r.relation_type = ?2",
                )
                .map_err(|e| format!("get_relations_by_type: {}", e))?;

            let rows = stmt
                .query_map(params![bundle_path, relation_type], |row| {
                    Ok(RelationRow {
                        bundle_path: row.get(0)?,
                        relation_type: row.get(1)?,
                        source_path_id: row.get(2)?,
                        target_path_id: row.get(3)?,
                        source_name: String::new(),
                        target_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        file_id: row.get(5)?,
                        field_path: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                        target_bundle_path: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
                    })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    pub fn find_relations_by_bundle_and_source(
        &self,
        bundle_path: &str,
        relation_type: &str,
        source_path_id: i64,
    ) -> Result<Vec<RelationRow>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, r.relation_type, r.source_path_id,
                            r.target_path_id, r.target_name, r.file_id,
                            r.field_path, r.target_bundle_path
                     FROM relations r
                     JOIN bundles b ON b.id = r.bundle_id
                     WHERE b.bundle_path = ?1 AND r.relation_type = ?2
                           AND r.source_path_id = ?3
                     ORDER BY r.id",
                )
                .map_err(|e| format!("find_relations_by_bundle_and_source: {}", e))?;

            let rows = stmt
                .query_map(params![bundle_path, relation_type, source_path_id], |row| {
                    Ok(RelationRow {
                        bundle_path: row.get(0)?,
                        relation_type: row.get(1)?,
                        source_path_id: row.get(2)?,
                        target_path_id: row.get(3)?,
                        source_name: String::new(),
                        target_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        file_id: row.get(5)?,
                        field_path: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                        target_bundle_path: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
                    })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    pub fn find_relations_by_bundle_and_target(
        &self,
        bundle_path: &str,
        relation_type: &str,
        target_path_id: i64,
    ) -> Result<Vec<RelationRow>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, r.relation_type, r.source_path_id,
                            r.target_path_id, r.target_name, r.file_id,
                            r.field_path, r.target_bundle_path
                     FROM relations r
                     JOIN bundles b ON b.id = r.bundle_id
                     WHERE b.bundle_path = ?1 AND r.relation_type = ?2
                           AND r.target_path_id = ?3
                     ORDER BY r.id",
                )
                .map_err(|e| format!("find_relations_by_bundle_and_target: {}", e))?;

            let rows = stmt
                .query_map(params![bundle_path, relation_type, target_path_id], |row| {
                    Ok(RelationRow {
                        bundle_path: row.get(0)?,
                        relation_type: row.get(1)?,
                        source_path_id: row.get(2)?,
                        target_path_id: row.get(3)?,
                        source_name: String::new(),
                        target_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        file_id: row.get(5)?,
                        field_path: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                        target_bundle_path: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
                    })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    pub fn find_relations_by_target(
        &self,
        target_bundle_path: &str,
        target_path_id: i64,
    ) -> Result<Vec<RelationRow>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, r.relation_type, r.source_path_id,
                            r.target_path_id, r.target_name, r.file_id,
                            r.field_path, r.target_bundle_path
                     FROM relations r
                     JOIN bundles b ON b.id = r.bundle_id
                     WHERE r.target_bundle_path = ?1 AND r.target_path_id = ?2",
                )
                .map_err(|e| format!("find_relations_by_target: {}", e))?;

            let rows = stmt
                .query_map(params![target_bundle_path, target_path_id], |row| {
                    Ok(RelationRow {
                        bundle_path: row.get(0)?,
                        relation_type: row.get(1)?,
                        source_path_id: row.get(2)?,
                        target_path_id: row.get(3)?,
                        source_name: String::new(),
                        target_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        file_id: row.get(5)?,
                        field_path: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                        target_bundle_path: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
                    })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    // ----------------------------------------------------------
    // Class stats
    // ----------------------------------------------------------

    pub fn class_stats_for_all_assets(&self) -> Result<Vec<(String, usize)>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT class_name, COUNT(*) as cnt
                     FROM assets
                     GROUP BY class_name
                     ORDER BY cnt DESC",
                )
                .map_err(|e| format!("class_stats_for_all_assets: {}", e))?;

            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    pub fn class_stats_for_bundle(
        &self,
        bundle_path: &str,
    ) -> Result<Vec<(String, usize)>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT a.class_name, COUNT(*) as cnt
                     FROM assets a
                     JOIN bundles b ON b.id = a.bundle_id
                     WHERE b.bundle_path = ?1
                     GROUP BY a.class_name
                     ORDER BY cnt DESC",
                )
                .map_err(|e| format!("class_stats_for_bundle: {}", e))?;

            let rows = stmt
                .query_map(params![bundle_path], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    // ----------------------------------------------------------
    // Asset query (for query_service)
    // ----------------------------------------------------------

    #[allow(dead_code)]
    pub fn count_assets_for_all_assets(
        &self,
        class_names: &[String],
        search: &str,
    ) -> Result<usize, String> {
        self.with_conn(|conn| {
            let (where_clause, param_values) = build_asset_where(class_names, search);

            let sql = format!(
                "SELECT COUNT(*) FROM assets a
                 JOIN bundles b ON b.id = a.bundle_id
                 WHERE {}",
                where_clause
            );

            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| format!("count_assets: {}", e))?;

            let params_refs: Vec<&dyn rusqlite::types::ToSql> = param_values
                .iter()
                .map(|v| v as &dyn rusqlite::types::ToSql)
                .collect();

            let count = stmt
                .query_row(params_refs.as_slice(), |row| {
                    row.get::<_, i64>(0).map(|v| v as usize)
                })
                .map_err(|e| format!("count query: {}", e))?;

            Ok(count)
        })
    }

    #[allow(dead_code)]
    pub fn query_assets_for_all_assets(
        &self,
        class_names: &[String],
        search: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<AssetDisplayRow>, String> {
        self.with_conn(|conn| {
            let (where_clause, param_values) = build_asset_where(class_names, search);

            let sql = format!(
                "SELECT b.bundle_path, a.path_id, a.class_id, a.class_name,
                        a.asset_name, a.byte_size, a.asset_path
                 FROM assets a
                 JOIN bundles b ON b.id = a.bundle_id
                 WHERE {}
                 ORDER BY a.class_name, a.asset_name
                 LIMIT {} OFFSET {}",
                where_clause, limit, offset
            );

            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| format!("query_assets: {}", e))?;

            let params_refs: Vec<&dyn rusqlite::types::ToSql> = param_values
                .iter()
                .map(|v| v as &dyn rusqlite::types::ToSql)
                .collect();

            let rows = stmt
                .query_map(params_refs.as_slice(), |row| {
                    let asset = AssetRow {
                        bundle_path: row.get(0)?,
                        path_id: row.get(1)?,
                        class_id: row.get(2)?,
                        class_name: row.get(3)?,
                        asset_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        byte_size: row.get::<_, i64>(5)? as u32,
                    };
                    let asset_path: String = row.get::<_, Option<String>>(6)?.unwrap_or_default();
                    Ok(AssetDisplayRow { asset, asset_path })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    // ----------------------------------------------------------
    // Texture candidate search
    // ----------------------------------------------------------

    pub fn search_texture_candidates_by_text(
        &self,
        search_terms: &[String],
        limit: usize,
    ) -> Result<Vec<TextureCandidateRow>, String> {
        let terms = search_terms
            .iter()
            .map(|term| term.trim())
            .filter(|term| !term.is_empty())
            .collect::<Vec<_>>();
        if terms.is_empty() {
            return Ok(Vec::new());
        }

        self.with_conn(|conn| {
            let mut conditions = Vec::new();
            let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            for term in &terms {
                let idx = params_vec.len() + 1;
                conditions.push(format!(
                    "(a.asset_name LIKE ?{idx} OR a.asset_path LIKE ?{idx})"
                ));
                params_vec.push(Box::new(format!("%{}%", term)));
            }
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT b.bundle_path, a.path_id, a.class_id, a.class_name,
                            a.asset_name, a.byte_size, a.asset_path
                     FROM assets a
                     JOIN bundles b ON b.id = a.bundle_id
                     WHERE a.class_name IN ('Texture2D', 'Sprite')
                       AND {}
                     ORDER BY a.asset_name
                     LIMIT ?{}",
                    conditions.join(" AND "),
                    params_vec.len() + 1
                ))
                .map_err(|e| format!("search_texture_candidates: {}", e))?;
            params_vec.push(Box::new(limit as i64));
            let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                params_vec.iter().map(|param| param.as_ref()).collect();

            let rows = stmt
                .query_map(params_refs.as_slice(), |row| {
                    let asset = AssetRow {
                        bundle_path: row.get(0)?,
                        path_id: row.get(1)?,
                        class_id: row.get(2)?,
                        class_name: row.get(3)?,
                        asset_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        byte_size: row.get::<_, i64>(5)? as u32,
                    };
                    let asset_path: String = row.get::<_, Option<String>>(6)?.unwrap_or_default();
                    Ok(TextureCandidateRow { asset, asset_path })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    pub fn search_texture_candidates_by_prefixes(
        &self,
        prefixes: &[String],
        limit: usize,
    ) -> Result<Vec<TextureCandidateRow>, String> {
        let prefixes = prefixes
            .iter()
            .map(|prefix| prefix.trim())
            .filter(|prefix| !prefix.is_empty())
            .collect::<Vec<_>>();
        if prefixes.is_empty() {
            return Ok(Vec::new());
        }

        self.with_conn(|conn| {
            let mut conditions = Vec::new();
            let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            for prefix in &prefixes {
                let name_idx = params_vec.len() + 1;
                let slash_path_idx = params_vec.len() + 2;
                let backslash_path_idx = params_vec.len() + 3;
                conditions.push(format!(
                    "(LOWER(a.asset_name) LIKE LOWER(?{name_idx}) ESCAPE '\\'
                      OR LOWER(a.asset_path) LIKE LOWER(?{slash_path_idx}) ESCAPE '\\'
                      OR LOWER(a.asset_path) LIKE LOWER(?{backslash_path_idx}) ESCAPE '\\')"
                ));
                let escaped = escape_like_pattern(prefix);
                params_vec.push(Box::new(format!("{}%", escaped)));
                params_vec.push(Box::new(format!("%/{}%", escaped)));
                params_vec.push(Box::new(format!("%\\\\{}%", escaped)));
            }
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT b.bundle_path, a.path_id, a.class_id, a.class_name,
                            a.asset_name, a.byte_size, a.asset_path
                     FROM assets a
                     JOIN bundles b ON b.id = a.bundle_id
                     WHERE a.class_name IN ('Texture2D', 'Sprite')
                       AND ({})
                     ORDER BY a.asset_name
                     LIMIT ?{}",
                    conditions.join(" OR "),
                    params_vec.len() + 1
                ))
                .map_err(|e| format!("search_texture_candidates_by_prefixes: {}", e))?;
            params_vec.push(Box::new(limit as i64));
            let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                params_vec.iter().map(|param| param.as_ref()).collect();

            let rows = stmt
                .query_map(params_refs.as_slice(), |row| {
                    let asset = AssetRow {
                        bundle_path: row.get(0)?,
                        path_id: row.get(1)?,
                        class_id: row.get(2)?,
                        class_name: row.get(3)?,
                        asset_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        byte_size: row.get::<_, i64>(5)? as u32,
                    };
                    let asset_path: String = row.get::<_, Option<String>>(6)?.unwrap_or_default();
                    Ok(TextureCandidateRow { asset, asset_path })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    pub fn search_material_candidates_by_identity(
        &self,
        identities: &[String],
        limit: usize,
    ) -> Result<Vec<AssetDisplayRow>, String> {
        let identities = identities
            .iter()
            .map(|identity| identity.trim())
            .filter(|identity| !identity.is_empty())
            .collect::<Vec<_>>();
        if identities.is_empty() {
            return Ok(Vec::new());
        }

        self.with_conn(|conn| {
            let mut conditions = Vec::new();
            let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            for identity in &identities {
                let name_idx = params_vec.len() + 1;
                let slash_path_idx = params_vec.len() + 2;
                let backslash_path_idx = params_vec.len() + 3;
                conditions.push(format!(
                    "(LOWER(a.asset_name) = LOWER(?{name_idx})
                      OR LOWER(a.asset_path) LIKE LOWER(?{slash_path_idx}) ESCAPE '\\'
                      OR LOWER(a.asset_path) LIKE LOWER(?{backslash_path_idx}) ESCAPE '\\')"
                ));
                let escaped = escape_like_pattern(identity);
                params_vec.push(Box::new(identity.to_string()));
                params_vec.push(Box::new(format!("%/{}.%", escaped)));
                params_vec.push(Box::new(format!("%\\\\{}.%", escaped)));
            }

            let mut stmt = conn
                .prepare(&format!(
                    "SELECT b.bundle_path, a.path_id, a.class_id, a.class_name,
                            a.asset_name, a.byte_size, a.asset_path
                     FROM assets a
                     JOIN bundles b ON b.id = a.bundle_id
                     WHERE a.class_name = 'Material'
                       AND ({})
                     ORDER BY a.asset_name
                     LIMIT ?{}",
                    conditions.join(" OR "),
                    params_vec.len() + 1
                ))
                .map_err(|e| format!("search_material_candidates_by_identity: {}", e))?;
            params_vec.push(Box::new(limit as i64));
            let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                params_vec.iter().map(|param| param.as_ref()).collect();

            let rows = stmt
                .query_map(params_refs.as_slice(), |row| {
                    let asset = AssetRow {
                        bundle_path: row.get(0)?,
                        path_id: row.get(1)?,
                        class_id: row.get(2)?,
                        class_name: row.get(3)?,
                        asset_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        byte_size: row.get::<_, i64>(5)? as u32,
                    };
                    let asset_path: String = row.get::<_, Option<String>>(6)?.unwrap_or_default();
                    Ok(AssetDisplayRow { asset, asset_path })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    pub fn find_animation_clips_by_text_tokens(
        &self,
        tokens: &[String],
        bundle_path: Option<&str>,
    ) -> Result<Vec<AssetDisplayRow>, String> {
        let terms = tokens
            .iter()
            .map(|token| token.trim())
            .filter(|token| !token.is_empty())
            .collect::<Vec<_>>();
        if terms.is_empty() {
            return Ok(Vec::new());
        }

        self.with_conn(|conn| {
            let mut conditions = Vec::new();
            let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            for term in &terms {
                let idx = params_vec.len() + 1;
                conditions.push(format!(
                    "(a.asset_name LIKE ?{idx} OR a.asset_path LIKE ?{idx})"
                ));
                params_vec.push(Box::new(format!("%{}%", term)));
            }
            if let Some(bundle_path) = bundle_path.filter(|value| !value.trim().is_empty()) {
                let idx = params_vec.len() + 1;
                conditions.push(format!("b.bundle_path = ?{idx}"));
                params_vec.push(Box::new(bundle_path.to_string()));
            }
            let where_clause = conditions.join(" AND ");
            let sql = format!(
                "SELECT b.bundle_path, a.path_id, a.class_id, a.class_name,
                        a.asset_name, a.byte_size, a.asset_path
                 FROM assets a
                 JOIN bundles b ON b.id = a.bundle_id
                 WHERE a.class_name = 'AnimationClip'
                   AND ({})
                 ORDER BY a.asset_name
                 LIMIT 256",
                where_clause
            );

            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| format!("find_animation_clips: {}", e))?;
            let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                params_vec.iter().map(|p| p.as_ref()).collect();

            let rows = stmt
                .query_map(params_refs.as_slice(), |row| {
                    let asset = AssetRow {
                        bundle_path: row.get(0)?,
                        path_id: row.get(1)?,
                        class_id: row.get(2)?,
                        class_name: row.get(3)?,
                        asset_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        byte_size: row.get::<_, i64>(5)? as u32,
                    };
                    Ok(AssetDisplayRow {
                        asset,
                        asset_path: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                    })
                })
                .map_err(|e| format!("query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row: {}", e))?;

            Ok(rows)
        })
    }

    // ----------------------------------------------------------
    // Delete / clear
    // ----------------------------------------------------------

    #[allow(dead_code)]
    pub fn clear(&self) -> Result<(), String> {
        self.with_conn(|conn| {
            Self::create_asset_search_table(conn).ok();
            conn.execute_batch(
                "DELETE FROM relations;
                 DELETE FROM asset_containers;
                 DELETE FROM asset_externals;
                 DELETE FROM asset_internal_names;
                 DELETE FROM assets;
                 DELETE FROM bundles;
                 DELETE FROM build_meta;
                 DELETE FROM asset_search_trigram_fts;",
            )
            .map_err(|e| format!("clear: {}", e))?;

            // Re-create indexes
            for sql in BUILD_INDEXES_SQL {
                conn.execute(sql, []).ok();
            }
            Ok(())
        })
    }

    pub fn delete_bundle_data(&self, bundle_path: &str) -> Result<(), String> {
        self.with_conn(|conn| {
            // Get bundle ID
            let bundle_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM bundles WHERE bundle_path = ?1",
                    params![bundle_path],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| format!("find bundle: {}", e))?;

            if let Some(bid) = bundle_id {
                Self::create_asset_search_table(conn).ok();
                conn.execute(
                    "DELETE FROM asset_search_trigram_fts
                     WHERE asset_rowid IN (SELECT rowid FROM assets WHERE bundle_id = ?1)",
                    params![bid],
                )
                .ok();
                conn.execute("DELETE FROM relations WHERE bundle_id = ?1", params![bid])
                    .map_err(|e| format!("delete relations: {}", e))?;
                conn.execute(
                    "DELETE FROM asset_containers WHERE bundle_id = ?1",
                    params![bid],
                )
                .map_err(|e| format!("delete containers: {}", e))?;
                conn.execute(
                    "DELETE FROM asset_externals WHERE bundle_id = ?1",
                    params![bid],
                )
                .map_err(|e| format!("delete externals: {}", e))?;
                conn.execute(
                    "DELETE FROM asset_internal_names WHERE bundle_id = ?1",
                    params![bid],
                )
                .map_err(|e| format!("delete internal_names: {}", e))?;
                conn.execute("DELETE FROM assets WHERE bundle_id = ?1", params![bid])
                    .map_err(|e| format!("delete assets: {}", e))?;
                conn.execute("DELETE FROM bundles WHERE id = ?1", params![bid])
                    .map_err(|e| format!("delete bundle: {}", e))?;
                conn.execute(
                    "DELETE FROM build_meta WHERE key = ?1",
                    params![ASSET_SEARCH_META_KEY],
                )
                .ok();
            }
            Ok(())
        })
    }

    // ----- build meta helpers -----

    pub fn update_build_meta(&self, key: &str, value: &str) -> Result<(), String> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO build_meta (key, value) VALUES (?1, ?2)",
                params![key, value],
            )
            .map_err(|e| format!("update_build_meta: {}", e))?;
            Ok(())
        })
    }

    pub fn get_build_meta_i64(&self, key: &str) -> Result<i64, String> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT value FROM build_meta WHERE key = ?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| format!("get_build_meta_i64: {}", e))?
            .unwrap_or_default()
            .parse::<i64>()
            .map_err(|e| format!("parse build_meta '{key}': {e}"))
        })
    }

    pub fn get_build_meta_bool(&self, key: &str) -> Result<bool, String> {
        self.with_conn(|conn| {
            let value = conn
                .query_row(
                    "SELECT value FROM build_meta WHERE key = ?1",
                    params![key],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| format!("get_build_meta_bool: {}", e))?
                .unwrap_or_default();
            match value.trim().to_ascii_lowercase().as_str() {
                "" => Ok(false),
                "1" | "true" | "yes" => Ok(true),
                "0" | "false" | "no" => Ok(false),
                other => Err(format!("parse build_meta '{key}': invalid bool '{other}'")),
            }
        })
    }

    pub fn optimize(&self) -> Result<(), String> {
        self.with_conn(|conn| {
            conn.execute_batch("PRAGMA optimize;")
                .map_err(|e| format!("optimize: {}", e))
        })
    }

    // ----- relation queries -----

    pub fn find_relations_by_type_prefix(
        &self,
        bundle_path: &str,
        prefix: &str,
    ) -> Result<Vec<RelationRow>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT b.bundle_path, r.relation_type, r.source_path_id, r.target_path_id,
                            r.target_name, r.file_id, r.field_path,
                            r.target_bundle_path
                     FROM relations r
                     JOIN bundles b ON b.id = r.bundle_id
                     WHERE b.bundle_path = ?1 AND r.relation_type LIKE ?2",
                )
                .map_err(|e| format!("find_relations_by_type_prefix: {}", e))?;
            let pattern = format!("{}%", prefix);
            let rows = stmt
                .query_map(params![bundle_path, pattern], |row| {
                    Ok(RelationRow {
                        bundle_path: row.get(0)?,
                        relation_type: row.get(1)?,
                        source_path_id: row.get(2)?,
                        target_path_id: row.get(3)?,
                        source_name: String::new(),
                        target_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        file_id: row.get::<_, i32>(5)?,
                        field_path: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                        target_bundle_path: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
                    })
                })
                .map_err(|e| format!("find_relations_by_type_prefix query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("find_relations_by_type_prefix row: {}", e))?;
            Ok(rows)
        })
    }

    // ----- paginated asset queries -----

    pub fn count_assets(
        &self,
        class_name: Option<&str>,
        search: Option<&str>,
        _cancel_token: &std::sync::atomic::AtomicBool,
    ) -> Result<usize, String> {
        self.with_conn(|conn| {
            let class_list = class_name.map(|c| vec![c.to_string()]).unwrap_or_default();
            let search_str = search.unwrap_or("");
            let (where_clause, params) = build_asset_where_with_opt(&class_list, search_str, None);
            let sql = format!(
                "SELECT COUNT(*) FROM assets a
                 JOIN bundles b ON b.id = a.bundle_id
                 WHERE {}",
                where_clause
            );
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            let count: i64 = conn
                .query_row(&sql, param_refs.as_slice(), |row| row.get(0))
                .map_err(|e| format!("count_assets: {}", e))?;
            Ok(count as usize)
        })
    }

    pub fn query_assets(
        &self,
        class_name: Option<&str>,
        search: Option<&str>,
        offset: usize,
        limit: usize,
        _cancel_token: &std::sync::atomic::AtomicBool,
    ) -> Result<Vec<AssetDisplayRow>, String> {
        self.with_conn(|conn| {
            let class_list = class_name.map(|c| vec![c.to_string()]).unwrap_or_default();
            let search_str = search.unwrap_or("");
            let (where_clause, params) = build_asset_where_with_opt(&class_list, search_str, None);
            let sql = format!(
                "SELECT a.bundle_id, a.path_id, a.class_id, a.class_name,
                        a.asset_name, a.asset_path, a.byte_size,
                        b.bundle_path
                 FROM assets a
                 JOIN bundles b ON b.id = a.bundle_id
                 WHERE {}
                 ORDER BY a.class_name, a.asset_name
                 LIMIT ?{} OFFSET ?{}",
                where_clause,
                params.len() + 1,
                params.len() + 2
            );
            let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = params;
            all_params.push(Box::new(limit as i64));
            all_params.push(Box::new(offset as i64));
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                all_params.iter().map(|p| p.as_ref()).collect();
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| format!("query_assets: {}", e))?;
            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    let bundle_path: String = row.get(7)?;
                    let asset = AssetRow {
                        bundle_path: bundle_path.clone(),
                        path_id: row.get(1)?,
                        class_id: row.get(2)?,
                        class_name: row.get(3)?,
                        asset_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        byte_size: row.get::<_, i64>(6)? as u32,
                    };
                    Ok(AssetDisplayRow {
                        asset,
                        asset_path: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    })
                })
                .map_err(|e| format!("query_assets query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("query_assets row: {}", e))?;
            Ok(rows)
        })
    }

    pub fn count_assets_by_options(
        &self,
        options: &MapAssetQueryOptions,
        _cancel_token: &std::sync::atomic::AtomicBool,
    ) -> Result<usize, String> {
        self.with_conn(|conn| {
            let use_search_index = Self::is_asset_search_index_ready(conn);
            let (where_clause, params) = build_asset_where_from_options(options, use_search_index);
            let sql = format!(
                "SELECT COUNT(*) FROM assets a
                 JOIN bundles b ON b.id = a.bundle_id
                 WHERE {}",
                where_clause
            );
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|param| param.as_ref()).collect();
            let count: i64 = conn
                .query_row(&sql, param_refs.as_slice(), |row| row.get(0))
                .map_err(|e| format!("count_assets_by_options: {}", e))?;
            Ok(count as usize)
        })
    }

    pub fn query_assets_by_options(
        &self,
        options: &MapAssetQueryOptions,
        _cancel_token: &std::sync::atomic::AtomicBool,
    ) -> Result<Vec<AssetDisplayRow>, String> {
        self.with_conn(|conn| {
            let use_search_index = Self::is_asset_search_index_ready(conn);
            let (where_clause, mut params) =
                build_asset_where_from_options(options, use_search_index);
            let order_by = build_asset_order_by(options);
            let sql = format!(
                "SELECT a.bundle_id, a.path_id, a.class_id, a.class_name,
                        a.asset_name, a.asset_path, a.byte_size,
                        b.bundle_path
                 FROM assets a
                 JOIN bundles b ON b.id = a.bundle_id
                 WHERE {}
                 ORDER BY {}
                 LIMIT ?{} OFFSET ?{}",
                where_clause,
                order_by,
                params.len() + 1,
                params.len() + 2
            );
            params.push(Box::new(options.limit.max(1) as i64));
            params.push(Box::new(options.offset as i64));
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|param| param.as_ref()).collect();
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| format!("query_assets_by_options: {}", e))?;
            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    let bundle_path: String = row.get(7)?;
                    let asset = AssetRow {
                        bundle_path: bundle_path.clone(),
                        path_id: row.get(1)?,
                        class_id: row.get(2)?,
                        class_name: row.get(3)?,
                        asset_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        byte_size: row.get::<_, i64>(6)? as u32,
                    };
                    Ok(AssetDisplayRow {
                        asset,
                        asset_path: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    })
                })
                .map_err(|e| format!("query_assets_by_options query: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("query_assets_by_options row: {}", e))?;
            Ok(rows)
        })
    }

    // ----- single-bundle write operations -----

    pub fn insert_bundle(&self, rows: &MapBundleWriteRows) -> Result<usize, String> {
        self.create_tables()?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO bundles (bundle_path, file_name, file_size,
                 modified_ms, unity_version, md5)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    rows.bundle_path,
                    std::path::Path::new(&rows.bundle_path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(""),
                    rows.file_size as i64,
                    rows.modified_ms as i64,
                    rows.unity_version,
                    rows.md5,
                ],
            )
            .map_err(|e| format!("insert bundle: {}", e))?;

            let bundle_id: i64 = conn.last_insert_rowid();
            let container_path_by_id = rows
                .containers
                .iter()
                .map(|row| (row.path_id, row.asset_path.as_str()))
                .collect::<std::collections::HashMap<_, _>>();

            let mut count = 1usize;
            for asset in &rows.assets {
                conn.execute(
                    "INSERT OR REPLACE INTO assets (bundle_id, path_id, class_id,
                     class_name, asset_name, asset_path, byte_size)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        bundle_id,
                        asset.path_id,
                        asset.class_id,
                        asset.class_name,
                        asset.asset_name,
                        container_path_by_id
                            .get(&asset.path_id)
                            .copied()
                            .unwrap_or(""),
                        asset.byte_size,
                    ],
                )
                .map_err(|e| format!("insert asset: {}", e))?;
                count += 1;
            }
            Self::index_bundle_assets_for_search(conn, bundle_id)?;
            for container in &rows.containers {
                conn.execute(
                    "INSERT INTO asset_containers (bundle_id, path_id, asset_path)
                     VALUES (?1, ?2, ?3)",
                    params![bundle_id, container.path_id, container.asset_path],
                )
                .map_err(|e| format!("insert container: {}", e))?;
            }
            for external in &rows.externals {
                conn.execute(
                    "INSERT INTO asset_externals (bundle_id, sf_index, file_id, path_name)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        bundle_id,
                        external.sf_index as i64,
                        external.file_id,
                        external.path_name
                    ],
                )
                .map_err(|e| format!("insert external: {}", e))?;
            }
            for internal_name in &rows.internal_names {
                conn.execute(
                    "INSERT INTO asset_internal_names (bundle_id, path_id, name, kind)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![bundle_id, 0, internal_name.name, internal_name.kind],
                )
                .map_err(|e| format!("insert internal_name: {}", e))?;
            }
            for relation in &rows.relations {
                conn.execute(
                    "INSERT INTO relations
                     (bundle_id, source_path_id, target_path_id, relation_type,
                      target_name, file_id, field_path, target_bundle_path)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        bundle_id,
                        relation.source_path_id,
                        relation.target_path_id,
                        relation.relation_type,
                        relation.target_name,
                        relation.file_id,
                        relation.field_path,
                        relation.target_bundle_path,
                    ],
                )
                .map_err(|e| format!("insert relation: {}", e))?;
            }
            Ok(count)
        })
    }

    pub fn delete_bundle(&self, bundle_path: &str) -> Result<(), String> {
        self.delete_bundle_data(bundle_path)
    }

    pub fn insert_or_replace_bundle(&self, rows: &MapBundleWriteRows) -> Result<(), String> {
        self.create_tables()?;
        self.delete_bundle_data(&rows.bundle_path)?;
        self.insert_bundle(rows)?;
        Ok(())
    }

    // ----- static helpers -----

    pub fn delete_db(workspace: &Path, cache_root: Option<&Path>) -> Result<(), String> {
        let db_path = Self::db_path(workspace, cache_root);
        Self::delete_db_path(&db_path)
    }

    fn delete_db_path(db_path: &Path) -> Result<(), String> {
        if db_path.exists() {
            std::fs::remove_file(&db_path)
                .map_err(|e| format!("delete db {}: {}", db_path.display(), e))?;
        }
        // Also delete WAL and SHM files
        let wal_path = db_path.with_extension("db-wal");
        if wal_path.exists() {
            std::fs::remove_file(&wal_path).ok();
        }
        let shm_path = db_path.with_extension("db-shm");
        if shm_path.exists() {
            std::fs::remove_file(&shm_path).ok();
        }
        Ok(())
    }
}

// ============================================================
// Shared helper - build WHERE clause for asset queries
// ============================================================

#[allow(dead_code)]
fn build_asset_where(
    class_names: &[String],
    search: &str,
) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
    let mut conditions: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if !class_names.is_empty() {
        let placeholders: Vec<String> = class_names
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        conditions.push(format!("a.class_name IN ({})", placeholders.join(", ")));
        for cn in class_names {
            params.push(Box::new(cn.clone()));
        }
    }

    if !search.is_empty() {
        let idx = params.len() + 1;
        conditions.push(format!(
            "(a.asset_name LIKE ?{idx} OR a.asset_path LIKE ?{idx})"
        ));
        params.push(Box::new(format!("%{}%", search)));
    }

    let where_clause = if conditions.is_empty() {
        "1=1".to_string()
    } else {
        conditions.join(" AND ")
    };

    (where_clause, params)
}

fn build_asset_where_with_opt(
    class_names: &[String],
    search: &str,
    bundle_path: Option<&str>,
) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
    let mut conditions: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if !class_names.is_empty() {
        let placeholders: Vec<String> = class_names
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        conditions.push(format!("a.class_name IN ({})", placeholders.join(", ")));
        for cn in class_names {
            params.push(Box::new(cn.clone()));
        }
    }

    if !search.is_empty() {
        let idx = params.len() + 1;
        conditions.push(format!(
            "(a.asset_name LIKE ?{idx} OR a.asset_path LIKE ?{idx})"
        ));
        params.push(Box::new(format!("%{}%", search)));
    }

    if let Some(bp) = bundle_path {
        let idx = params.len() + 1;
        conditions.push(format!("b.bundle_path = ?{idx}"));
        params.push(Box::new(bp.to_string()));
    }

    let where_clause = if conditions.is_empty() {
        "1=1".to_string()
    } else {
        conditions.join(" AND ")
    };

    (where_clause, params)
}

fn build_asset_where_from_options(
    options: &MapAssetQueryOptions,
    use_search_index: bool,
) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
    let mut conditions: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut fts_terms: Vec<String> = Vec::new();

    let class_names = options
        .class_names
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    if !class_names.is_empty() {
        let placeholders = (0..class_names.len())
            .map(|idx| format!("?{}", params.len() + idx + 1))
            .collect::<Vec<_>>();
        conditions.push(format!("a.class_name IN ({})", placeholders.join(", ")));
        for class_name in class_names {
            params.push(Box::new(class_name.to_string()));
        }
    }

    let search = options.search.trim();
    if !search.is_empty() {
        if let Some(query) = asset_search_fts_query(search) {
            fts_terms.push(query);
        }
        let idx = params.len() + 1;
        conditions.push(format!(
            "(LOWER(a.asset_name) LIKE LOWER(?{idx})
              OR LOWER(a.class_name) LIKE LOWER(?{idx})
              OR LOWER(a.asset_path) LIKE LOWER(?{idx})
              OR LOWER(b.bundle_path) LIKE LOWER(?{idx}))"
        ));
        params.push(Box::new(format!("%{}%", search)));
    }

    if let Some(bundle_path) = options
        .bundle_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let idx = params.len() + 1;
        conditions.push(format!("b.bundle_path = ?{idx}"));
        params.push(Box::new(bundle_path.to_string()));
    }

    if let Some(name_query) = options
        .name_query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let idx = params.len() + 1;
        let normalized = name_query.to_string();
        match options
            .name_match_mode
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("exact") | Some("equals") => {
                conditions.push(format!("LOWER(a.asset_name) = LOWER(?{idx})"));
                params.push(Box::new(normalized));
            }
            Some("starts_with") => {
                if let Some(query) = asset_search_fts_query(name_query) {
                    fts_terms.push(query);
                }
                conditions.push(format!("LOWER(a.asset_name) LIKE LOWER(?{idx})"));
                params.push(Box::new(format!("{}%", normalized)));
            }
            Some("ends_with") => {
                conditions.push(format!("LOWER(a.asset_name) LIKE LOWER(?{idx})"));
                params.push(Box::new(format!("%{}", normalized)));
            }
            _ => {
                if let Some(query) = asset_search_fts_query(name_query) {
                    fts_terms.push(query);
                }
                conditions.push(format!(
                    "(LOWER(a.asset_name) LIKE LOWER(?{idx}) OR LOWER(a.asset_path) LIKE LOWER(?{idx}))"
                ));
                params.push(Box::new(format!("%{}%", normalized)));
            }
        }
    }

    if let Some(include_queries) = options.name_include_queries.as_ref() {
        for include in include_queries
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            if let Some(query) = asset_search_fts_query(include) {
                fts_terms.push(query);
            }
            let idx = params.len() + 1;
            conditions.push(format!(
                "(LOWER(a.asset_name) LIKE LOWER(?{idx}) OR LOWER(a.asset_path) LIKE LOWER(?{idx}))"
            ));
            params.push(Box::new(format!("%{}%", include)));
        }
    }

    if let Some(exclude_query) = options
        .name_exclude_query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let idx = params.len() + 1;
        conditions.push(format!(
            "(LOWER(a.asset_name) NOT LIKE LOWER(?{idx}) AND LOWER(a.asset_path) NOT LIKE LOWER(?{idx}))"
        ));
        params.push(Box::new(format!("%{}%", exclude_query)));
    }

    if let Some(exclude_queries) = options.name_exclude_queries.as_ref() {
        for exclude in exclude_queries
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            let idx = params.len() + 1;
            conditions.push(format!(
                "(LOWER(a.asset_name) NOT LIKE LOWER(?{idx}) AND LOWER(a.asset_path) NOT LIKE LOWER(?{idx}))"
            ));
            params.push(Box::new(format!("%{}%", exclude)));
        }
    }

    if let Some(bundle_query) = options
        .bundle_query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(query) = asset_search_fts_query(bundle_query) {
            fts_terms.push(query);
        }
        let idx = params.len() + 1;
        conditions.push(format!("LOWER(b.bundle_path) LIKE LOWER(?{idx})"));
        params.push(Box::new(format!("%{}%", bundle_query)));
    }

    if let Some(path_id_query) = options
        .path_id_query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let idx = params.len() + 1;
        conditions.push(format!("CAST(a.path_id AS TEXT) LIKE ?{idx}"));
        params.push(Box::new(format!("%{}%", path_id_query)));
    }

    if let Some(min_size) = options.min_size {
        let idx = params.len() + 1;
        conditions.push(format!("a.byte_size >= ?{idx}"));
        params.push(Box::new(min_size as i64));
    }

    if let Some(max_size) = options.max_size {
        let idx = params.len() + 1;
        conditions.push(format!("a.byte_size <= ?{idx}"));
        params.push(Box::new(max_size as i64));
    }

    if use_search_index && !fts_terms.is_empty() {
        let idx = params.len() + 1;
        conditions.push(format!(
            "a.rowid IN (SELECT asset_rowid FROM asset_search_trigram_fts WHERE asset_search_trigram_fts MATCH ?{idx})"
        ));
        params.push(Box::new(fts_terms.join(" ")));
    }

    let where_clause = if conditions.is_empty() {
        "1=1".to_string()
    } else {
        conditions.join(" AND ")
    };

    (where_clause, params)
}

fn asset_search_fts_query(value: &str) -> Option<String> {
    let tokens = value
        .split(|ch: char| !is_asset_search_token_char(ch))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .filter(|token| token.len() >= 3)
        .map(|token| {
            let escaped = token.replace('"', "\"\"");
            format!("\"{}\"", escaped)
        })
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" AND "))
    }
}

fn escape_like_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' | '%' | '_' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn is_asset_search_token_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '.' | '/' | '\\' | '-' | ':')
}

fn build_asset_order_by(options: &MapAssetQueryOptions) -> &'static str {
    let desc = options
        .sort_direction
        .as_deref()
        .map(|value| value.trim().eq_ignore_ascii_case("desc"))
        .unwrap_or(false);
    match (
        options
            .sort_by
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase()),
        desc,
    ) {
        (Some(value), true) if value == "name" => {
            "a.asset_name DESC, a.class_name DESC, b.bundle_path DESC, a.path_id DESC"
        }
        (Some(value), true) if value == "bundle" => {
            "b.bundle_path DESC, a.class_name DESC, a.asset_name DESC, a.path_id DESC"
        }
        (Some(value), true) if value == "size" => {
            "a.byte_size DESC, a.class_name DESC, a.asset_name DESC, a.path_id DESC"
        }
        (Some(value), true) if value == "path_id" => {
            "a.path_id DESC, a.class_name DESC, a.asset_name DESC, b.bundle_path DESC"
        }
        (Some(value), false) if value == "name" => {
            "a.asset_name ASC, a.class_name ASC, b.bundle_path ASC, a.path_id ASC"
        }
        (Some(value), false) if value == "bundle" => {
            "b.bundle_path ASC, a.class_name ASC, a.asset_name ASC, a.path_id ASC"
        }
        (Some(value), false) if value == "size" => {
            "a.byte_size ASC, a.class_name ASC, a.asset_name ASC, a.path_id ASC"
        }
        (Some(value), false) if value == "path_id" => {
            "a.path_id ASC, a.class_name ASC, a.asset_name ASC, b.bundle_path ASC"
        }
        (_, true) => "a.class_name DESC, a.asset_name DESC, b.bundle_path DESC, a.path_id DESC",
        _ => "a.class_name ASC, a.asset_name ASC, b.bundle_path ASC, a.path_id ASC",
    }
}

impl SqliteBuildSession {
    pub fn create(workspace: &Path, cache_root: Option<&Path>) -> Result<Self, String> {
        SqliteStore::open(workspace, cache_root)?.begin_build_session()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::asset_map::asset_index::{
        AssetWriteRow, ContainerWriteRow, MapBundleWriteRows, RelationWriteRow,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("assetfinder_sqlite_store_{}_{}", name, suffix))
    }

    fn map_bundle(bundle_path: &str, assets: Vec<AssetWriteRow>) -> MapBundleWriteRows {
        MapBundleWriteRows {
            bundle_path: bundle_path.to_string(),
            md5: bundle_path.to_string(),
            file_size: 1,
            modified_ms: 1,
            unity_version: "2019.4.41f2".to_string(),
            asset_count: assets.len(),
            assets,
            containers: Vec::new(),
            externals: Vec::new(),
            internal_names: Vec::new(),
            relations: Vec::new(),
        }
    }

    fn asset(path_id: i64, class_name: &str, asset_name: &str) -> AssetWriteRow {
        AssetWriteRow {
            bundle_path: String::new(),
            path_id,
            class_id: 1,
            class_name: class_name.to_string(),
            asset_name: asset_name.to_string(),
            byte_size: 1,
        }
    }

    fn relation(relation_type: &str, source_path_id: i64, target_path_id: i64) -> RelationWriteRow {
        RelationWriteRow {
            bundle_path: String::new(),
            relation_type: relation_type.to_string(),
            source_path_id,
            target_path_id,
            source_name: String::new(),
            target_name: String::new(),
            file_id: 0,
            field_path: "m_Mesh".to_string(),
            target_bundle_path: String::new(),
        }
    }

    #[test]
    fn all_assets_contains_query_uses_trigram_search_index() {
        let workspace = temp_workspace("fts_contains");
        std::fs::create_dir_all(&workspace).unwrap();
        let store = SqliteStore::open(&workspace, None).unwrap();
        let bundle_path = workspace.join("bundle").to_string_lossy().to_string();
        store
            .insert_bundle(&map_bundle(
                &bundle_path,
                vec![
                    asset(1, "Animator", "ch_f_japan_onmyoji_lv_s11"),
                    asset(2, "Animator", "ch_f_other"),
                ],
            ))
            .unwrap();

        let options = MapAssetQueryOptions {
            search: String::new(),
            class_names: Vec::new(),
            bundle_path: None,
            name_query: None,
            name_match_mode: Some("contains".to_string()),
            name_include_queries: Some(vec!["japan_onmyoji".to_string()]),
            name_exclude_query: None,
            name_exclude_queries: None,
            bundle_query: None,
            path_id_query: None,
            min_size: None,
            max_size: None,
            offset: 0,
            limit: 20,
            sort_by: Some("name".to_string()),
            sort_direction: Some("asc".to_string()),
        };

        let rows = store
            .query_assets_by_options(&options, &std::sync::atomic::AtomicBool::new(false))
            .unwrap();
        let total = store
            .count_assets_by_options(&options, &std::sync::atomic::AtomicBool::new(false))
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].asset.asset_name, "ch_f_japan_onmyoji_lv_s11");

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn texture_prefix_query_escapes_underscore_and_matches_asset_path_basename() {
        let workspace = temp_workspace("texture_prefix");
        std::fs::create_dir_all(&workspace).unwrap();
        let store = SqliteStore::open(&workspace, None).unwrap();
        let bundle_path = workspace.join("bundle").to_string_lossy().to_string();
        let mut rows = map_bundle(
            &bundle_path,
            vec![
                asset(1, "Texture2D", "Cat_Parts_0013_Head_D_01_AMS"),
                asset(2, "Texture2D", "ChaHu_0001_D_01_AMS"),
                asset(3, "Texture2D", "CatXParts_0013_Head_D_01_AMS"),
            ],
        );
        rows.containers = vec![
            crate::common::asset_map::asset_index::ContainerWriteRow {
                bundle_path: String::new(),
                path_id: 1,
                asset_path:
                    "assets/artdata/character/npc/textures/cat_parts_0013_head_d_01_ams.tga"
                        .to_string(),
            },
            crate::common::asset_map::asset_index::ContainerWriteRow {
                bundle_path: String::new(),
                path_id: 2,
                asset_path: "assets/artdata/scene/textures/chahu_0001_d_01_ams.tga".to_string(),
            },
            crate::common::asset_map::asset_index::ContainerWriteRow {
                bundle_path: String::new(),
                path_id: 3,
                asset_path:
                    "assets/artdata/character/npc/textures/catxparts_0013_head_d_01_ams.tga"
                        .to_string(),
            },
        ];
        store.insert_bundle(&rows).unwrap();

        let found = store
            .search_texture_candidates_by_prefixes(&["cat_parts_0013_head".to_string()], 16)
            .unwrap();

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].asset.asset_name, "Cat_Parts_0013_Head_D_01_AMS");

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn material_identity_query_matches_exact_name_and_path_stem() {
        let workspace = temp_workspace("material_identity");
        std::fs::create_dir_all(&workspace).unwrap();
        let store = SqliteStore::open(&workspace, None).unwrap();
        let bundle_path = workspace.join("bundle").to_string_lossy().to_string();
        let mut rows = map_bundle(
            &bundle_path,
            vec![
                asset(1, "Material", "Cat_Parts_0013_Head"),
                asset(2, "Material", "ChaHu_0001"),
                asset(3, "Material", "CatXParts_0013_Head"),
            ],
        );
        rows.containers = vec![
            ContainerWriteRow {
                bundle_path: String::new(),
                path_id: 1,
                asset_path: "assets/artdata/character/materials/cat_parts_0013_head.mat"
                    .to_string(),
            },
            ContainerWriteRow {
                bundle_path: String::new(),
                path_id: 2,
                asset_path: "assets/artdata/character/materials/chahu_0001.mat".to_string(),
            },
            ContainerWriteRow {
                bundle_path: String::new(),
                path_id: 3,
                asset_path: "assets/artdata/character/materials/catxparts_0013_head.mat"
                    .to_string(),
            },
        ];
        store.insert_bundle(&rows).unwrap();

        let found = store
            .search_material_candidates_by_identity(&["cat_parts_0013_head".to_string()], 16)
            .unwrap();

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].asset.asset_name, "Cat_Parts_0013_Head");
        assert_eq!(
            found[0].asset_path,
            "assets/artdata/character/materials/cat_parts_0013_head.mat"
        );

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn build_session_flush_writes_relations_table() {
        let workspace = temp_workspace("build_relations");
        std::fs::create_dir_all(&workspace).unwrap();
        let store = SqliteStore::open(&workspace, None).unwrap();
        let bundle_path = workspace.join("bundle").to_string_lossy().to_string();

        let mut rows = map_bundle(
            &bundle_path,
            vec![
                asset(10, "SkinnedMeshRenderer", "renderer"),
                asset(20, "Mesh", "mesh"),
            ],
        );
        rows.relations = vec![relation("renderer_mesh", 10, 20)];

        let mut session = store.begin_build_session().unwrap();
        session.append_bundle(Arc::new(rows)).unwrap();
        session.flush().unwrap();
        drop(session);

        let relations = store
            .find_relations_by_bundle_and_source(&bundle_path, "renderer_mesh", 10)
            .unwrap();
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].target_path_id, 20);
        assert_eq!(relations[0].field_path, "m_Mesh");

        let _ = std::fs::remove_dir_all(workspace);
    }
}
