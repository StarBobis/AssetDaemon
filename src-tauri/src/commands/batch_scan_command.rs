/*
 * Batch type scan command -- scan Bundle file types in parallel and cache MD5.
 *
 * This file only contains #[tauri::command] functions; actual logic is delegated to BatchScanner.
 * Follows the separation principle: commands are separated from tool classes.
 */

use crate::common::scan::batch_scanner::BatchScanner;
use crate::common::scan::scan_types::*;
use crate::common::task::task_logger::TaskLogger;
use crate::common::task::task_runner::TaskRunner;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::ipc::Channel;
use tokio::task::JoinSet;
use tokio::time::{sleep, Duration};

/**
 * Batch scan Bundle file types.
 *
 * Uses parallel processing + MD5 caching to accelerate repeated scans.
 * Files whose MD5 is unchanged from cache are skipped.
 *
 * @param paths        List of file paths to scan
 * @param cached       Cached MD5 mapping {path: md5}
 * @param concurrency  Maximum concurrency (1 ~ 32)
 * @param on_progress  Progress callback Channel
 * @return             Type scan result for each file
 */
#[tauri::command]
pub async fn batch_scan_types(
    paths: Vec<String>,
    cached: HashMap<String, String>,
    concurrency: usize,
    on_progress: Channel<ScanProgress>,
) -> Result<Vec<FileTypeResult>, String> {
    let total = paths.len();
    let concurrency = concurrency.max(1).min(32);

    let ctx = TaskRunner::create("batch_scan", "batch scan");
    ctx.info(format!(
        "starting batch type scan: {} files (concurrency={})",
        total, concurrency
    ));
    let cancel_token = ctx.cancel_token();
    let cached = Arc::new(cached);

    let mut results = Vec::with_capacity(total);
    let mut done = 0usize;
    let mut pending = paths.into_iter();
    let mut workers = JoinSet::<Result<(String, FileTypeResult), String>>::new();

    loop {
        while workers.len() < concurrency && !cancel_token.load(Ordering::SeqCst) {
            let Some(path) = pending.next() else {
                break;
            };
            let cached_copy = cached.clone();
            let cancel = cancel_token.clone();
            workers.spawn_blocking(move || {
                if cancel.load(Ordering::SeqCst) {
                    return Err("Task cancelled".to_string());
                }
                let result =
                    BatchScanner::scan_single_file_cancelable(&path, &cached_copy, Some(&cancel))?;
                Ok((path, result))
            });
        }

        if cancel_token.load(Ordering::SeqCst) {
            workers.abort_all();
            ctx.warn(format!(
                "batch scan cancelled after {}/{} files",
                done, total
            ));
            return Err("Task cancelled".to_string());
        }

        if workers.is_empty() {
            break;
        }

        let join_result = tokio::select! {
            result = workers.join_next() => result,
            _ = sleep(Duration::from_millis(50)) => {
                if cancel_token.load(Ordering::SeqCst) {
                    workers.abort_all();
                    ctx.warn(format!("batch scan cancelled after {}/{} files", done, total));
                    return Err("Task cancelled".to_string());
                }
                continue;
            }
        };

        let Some(join_result) = join_result else {
            break;
        };
        match join_result {
            Ok(Ok((path, result))) => {
                done += 1;
                TaskLogger::progress(
                    ctx.task_id(),
                    "batch scan",
                    "scan",
                    done,
                    total,
                    &format!("scanned {}", path),
                );
                let _ = on_progress.send(ScanProgress {
                    done,
                    total,
                    current: path,
                });
                results.push(result);
            }
            Ok(Err(error)) if error.to_ascii_lowercase().contains("cancel") => {
                workers.abort_all();
                ctx.warn(format!(
                    "batch scan cancelled after {}/{} files",
                    done, total
                ));
                return Err("Task cancelled".to_string());
            }
            Ok(Err(error)) => {
                workers.abort_all();
                ctx.error(format!("scan worker error: {}", error));
                return Err(error);
            }
            Err(error) => {
                workers.abort_all();
                ctx.error(format!("scan thread error: {}", error));
                return Err(format!("scan task failed: {}", error));
            }
        }
    }

    let parsed = results.iter().filter(|r| !r.types.is_empty()).count();
    let skipped = total - parsed;
    ctx.success(format!(
        "batch scan complete: {} newly parsed, {} cache hits",
        parsed, skipped
    ));
    Ok(results)
}
