use crate::common::asset_map::asset_index::{AssetDatabase, BundleInfoRow, MapBundleWriteRows};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

const DDL_PARSE_CACHE: &str = "
    CREATE TABLE IF NOT EXISTS bundle_parse_cache (
        bundle_path     TEXT PRIMARY KEY,
        file_size       INTEGER NOT NULL,
        modified_ms     INTEGER NOT NULL,
        fingerprint     TEXT    NOT NULL,
        schema_version  INTEGER NOT NULL,
        rows_json       TEXT    NOT NULL
    );
";

const BASE_SCHEMA_VERSION: i64 = 2;
const PARSE_CACHE_SHARDS: usize = 3;

pub struct AssetMapParseCacheStore {
    db_paths: Vec<PathBuf>,
}

impl AssetMapParseCacheStore {
    pub fn open(workspace: &Path, cache_root: Option<&Path>) -> Result<Self, String> {
        let db_paths = Self::db_paths(workspace, cache_root);
        for db_path in &db_paths {
            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create parse cache dir: {}", e))?;
            }
        }
        let store = Self { db_paths };
        store.create_tables()?;
        Ok(store)
    }

    pub fn get_valid(
        &self,
        bundle_path: &str,
        file_size: u64,
        modified_ms: u64,
        fingerprint: &str,
        include_relations: bool,
    ) -> Result<Option<MapBundleWriteRows>, String> {
        let schema_version = Self::schema_version(include_relations);
        self.with_shard_conn(bundle_path, |conn| {
            let rows_json = conn
                .query_row(
                    "SELECT rows_json
                     FROM bundle_parse_cache
                     WHERE bundle_path = ?1
                       AND file_size = ?2
                       AND modified_ms = ?3
                       AND fingerprint = ?4
                       AND schema_version = ?5",
                    params![
                        bundle_path,
                        file_size as i64,
                        modified_ms as i64,
                        fingerprint,
                        schema_version,
                    ],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| format!("query parse cache: {}", e))?;

            rows_json
                .map(|value| {
                    serde_json::from_str::<MapBundleWriteRows>(&value)
                        .map_err(|e| format!("decode parse cache: {}", e))
                })
                .transpose()
        })
    }

    pub fn get_valid_many(
        &self,
        bundles: &[BundleInfoRow],
        include_relations: bool,
    ) -> Result<Vec<MapBundleWriteRows>, String> {
        if bundles.is_empty() {
            return Ok(Vec::new());
        }

        let schema_version = Self::schema_version(include_relations);
        let mut bundles_by_shard: Vec<Vec<&BundleInfoRow>> =
            (0..PARSE_CACHE_SHARDS).map(|_| Vec::new()).collect();
        for bundle in bundles {
            bundles_by_shard[Self::shard_index(&bundle.path)].push(bundle);
        }

        let mut cached_rows = Vec::new();
        for (shard_index, shard_bundles) in bundles_by_shard.into_iter().enumerate() {
            if shard_bundles.is_empty() {
                continue;
            }

            self.with_conn(shard_index, |conn| {
                let mut statement = conn
                    .prepare(
                        "SELECT rows_json
                         FROM bundle_parse_cache
                         WHERE bundle_path = ?1
                           AND file_size = ?2
                           AND modified_ms = ?3
                           AND fingerprint = ?4
                           AND schema_version = ?5",
                    )
                    .map_err(|e| format!("prepare parse cache batch query: {}", e))?;

                for bundle in shard_bundles {
                    let rows_json = statement
                        .query_row(
                            params![
                                bundle.path,
                                bundle.file_size as i64,
                                bundle.modified_ms as i64,
                                bundle.md5,
                                schema_version,
                            ],
                            |row| row.get::<_, String>(0),
                        )
                        .optional()
                        .map_err(|e| format!("query parse cache batch: {}", e))?;

                    if let Some(value) = rows_json {
                        cached_rows.push(
                            serde_json::from_str::<MapBundleWriteRows>(&value)
                                .map_err(|e| format!("decode parse cache batch: {}", e))?,
                        );
                    }
                }

                Ok(())
            })?;
        }

        Ok(cached_rows)
    }

    pub fn put_many_refs(
        &self,
        rows: &[&MapBundleWriteRows],
        include_relations: bool,
    ) -> Result<(), String> {
        if rows.is_empty() {
            return Ok(());
        }
        let schema_version = Self::schema_version(include_relations);

        let mut rows_by_shard: Vec<Vec<&MapBundleWriteRows>> =
            (0..PARSE_CACHE_SHARDS).map(|_| Vec::new()).collect();
        for row in rows {
            rows_by_shard[Self::shard_index(&row.bundle_path)].push(*row);
        }

        for (shard_index, shard_rows) in rows_by_shard.into_iter().enumerate() {
            if shard_rows.is_empty() {
                continue;
            }

            let mut conn = self.open_connection(shard_index)?;
            let tx = conn
                .transaction()
                .map_err(|e| format!("begin parse cache tx: {}", e))?;
            {
                let mut statement = tx
                    .prepare(
                        "INSERT OR REPLACE INTO bundle_parse_cache
                         (bundle_path, file_size, modified_ms, fingerprint, schema_version, rows_json)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    )
                    .map_err(|e| format!("prepare parse cache insert: {}", e))?;
                for row in shard_rows {
                    let rows_json = serde_json::to_string(row)
                        .map_err(|e| format!("encode parse cache: {}", e))?;
                    statement
                        .execute(params![
                            row.bundle_path,
                            row.file_size as i64,
                            row.modified_ms as i64,
                            row.md5,
                            schema_version,
                            rows_json,
                        ])
                        .map_err(|e| format!("write parse cache: {}", e))?;
                }
            }
            tx.commit()
                .map_err(|e| format!("commit parse cache tx: {}", e))?;
        }

        Ok(())
    }

    fn db_paths(workspace: &Path, cache_root: Option<&Path>) -> Vec<PathBuf> {
        let dir = match cache_root {
            Some(root) => {
                AssetDatabase::workspace_cache_dir(root, workspace).join("asset_parse_cache")
            }
            None => workspace.join("asset_parse_cache"),
        };
        (0..PARSE_CACHE_SHARDS)
            .map(|index| dir.join(format!("asset_parse_cache_{}.db", index)))
            .collect()
    }

    fn create_tables(&self) -> Result<(), String> {
        for shard_index in 0..self.db_paths.len() {
            self.with_conn(shard_index, |conn| {
                conn.execute_batch(DDL_PARSE_CACHE)
                    .map_err(|e| format!("create parse cache: {}", e))
            })?;
        }
        Ok(())
    }

    fn open_connection(&self, shard_index: usize) -> Result<Connection, String> {
        let db_path = self
            .db_paths
            .get(shard_index)
            .ok_or_else(|| format!("invalid parse cache shard {}", shard_index))?;
        let conn = Connection::open(db_path)
            .map_err(|e| format!("open parse cache {}: {}", db_path.display(), e))?;
        let _: String = conn
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .map_err(|e| format!("parse cache WAL: {}", e))?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")
            .map_err(|e| format!("parse cache sync: {}", e))?;
        Ok(conn)
    }

    fn with_shard_conn<T>(
        &self,
        bundle_path: &str,
        f: impl FnOnce(&Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_conn(Self::shard_index(bundle_path), f)
    }

    fn with_conn<T>(
        &self,
        shard_index: usize,
        f: impl FnOnce(&Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        let conn = self.open_connection(shard_index)?;
        f(&conn)
    }

    fn shard_index(bundle_path: &str) -> usize {
        let mut hash = 0xcbf29ce484222325_u64;
        for byte in bundle_path.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        (hash as usize) % PARSE_CACHE_SHARDS
    }

    fn schema_version(include_relations: bool) -> i64 {
        BASE_SCHEMA_VERSION * 10 + if include_relations { 1 } else { 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::asset_map::asset_index::AssetWriteRow;

    fn temp_workspace(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "assetfinder_parse_cache_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn rows(
        bundle_path: &str,
        file_size: u64,
        modified_ms: u64,
        fingerprint: &str,
    ) -> MapBundleWriteRows {
        MapBundleWriteRows {
            bundle_path: bundle_path.to_string(),
            md5: fingerprint.to_string(),
            file_size,
            modified_ms,
            unity_version: "2019.4.41f2".to_string(),
            asset_count: 1,
            assets: vec![AssetWriteRow {
                bundle_path: bundle_path.to_string(),
                path_id: 7,
                class_id: 28,
                class_name: "Texture2D".to_string(),
                asset_name: "cached_texture".to_string(),
                byte_size: 123,
            }],
            containers: Vec::new(),
            externals: Vec::new(),
            internal_names: Vec::new(),
            relations: Vec::new(),
        }
    }

    #[test]
    fn cache_returns_rows_only_for_matching_file_fingerprint() {
        let workspace = temp_workspace("fingerprint");
        std::fs::create_dir_all(&workspace).unwrap();
        let store = AssetMapParseCacheStore::open(&workspace, None).unwrap();
        let bundle_path = workspace.join("bundle_a").to_string_lossy().to_string();
        let cached_rows = rows(&bundle_path, 100, 200, "stat-v1:100:200:100");

        store.put_many_refs(&[&cached_rows], false).unwrap();

        let hit = store
            .get_valid(&bundle_path, 100, 200, "stat-v1:100:200:100", false)
            .unwrap()
            .expect("cache hit");
        assert_eq!(hit.assets[0].asset_name, "cached_texture");

        let different_scan_mode = store
            .get_valid(&bundle_path, 100, 200, "stat-v1:100:200:100", true)
            .unwrap();
        assert!(different_scan_mode.is_none());

        let miss = store
            .get_valid(&bundle_path, 100, 201, "stat-v1:100:201:100", false)
            .unwrap();
        assert!(miss.is_none());

        let _ = std::fs::remove_dir_all(workspace);
    }
}
