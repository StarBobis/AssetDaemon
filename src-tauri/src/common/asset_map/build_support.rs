use crate::common::bundle_file::bundle_types::BundleFileEntry;
use std::collections::VecDeque;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

pub struct AssetMapBuildSupport;

impl AssetMapBuildSupport {
    pub const DEFAULT_PARSE_MEMORY_LIMIT_BYTES: usize = 12 * 1024 * 1024 * 1024;
    pub const MAX_PARSE_MEMORY_LIMIT_BYTES: usize = 108 * 1024 * 1024 * 1024;
    const MIN_PARSE_MEMORY_LIMIT_BYTES: u64 = 1_u64 * 1024 * 1024 * 1024;
    const LARGE_BUNDLE_MEMORY_FLOOR_BYTES: usize = 64 * 1024 * 1024;
    const LARGE_BUNDLE_MEMORY_CAP_BYTES: usize = 512 * 1024 * 1024;
    const SMALL_BUNDLE_EXACT_MEMORY_THRESHOLD_BYTES: usize = 64 * 1024 * 1024;
    const MAX_PARSE_WORKER_COUNT: usize = 384;

    pub fn looks_like_unityfs_file(path: &str) -> Result<bool, String> {
        let mut file =
            fs::File::open(path).map_err(|e| format!("Unable to open file '{}': {}", path, e))?;
        let mut magic = [0u8; 7];
        match file.read_exact(&mut magic) {
            Ok(()) => Ok(&magic == b"UnityFS"),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
            Err(e) => Err(format!("Failed to read file header '{}': {}", path, e)),
        }
    }

    pub fn looks_like_serialized_file(path: &str, file_size: u64) -> Result<bool, String> {
        if file_size < 20 {
            return Ok(false);
        }

        let mut file =
            fs::File::open(path).map_err(|e| format!("Unable to open file '{}': {}", path, e))?;
        let mut header = [0u8; 48];
        let bytes_read = file
            .read(&mut header)
            .map_err(|e| format!("Failed to read file header '{}': {}", path, e))?;
        if bytes_read < 20 {
            return Ok(false);
        }

        let _metadata_size = u32::from_be_bytes(header[0..4].try_into().unwrap()) as u64;
        let mut serialized_file_size = u32::from_be_bytes(header[4..8].try_into().unwrap()) as u64;
        let version = u32::from_be_bytes(header[8..12].try_into().unwrap());
        let mut data_offset = u32::from_be_bytes(header[12..16].try_into().unwrap()) as u64;

        if !(1..=30).contains(&version) {
            return Ok(false);
        }
        if version >= 22 {
            if bytes_read < 40 {
                return Ok(false);
            }
            serialized_file_size = i64::from_be_bytes(header[24..32].try_into().unwrap()) as u64;
            data_offset = i64::from_be_bytes(header[32..40].try_into().unwrap()) as u64;
        }

        Ok(serialized_file_size == file_size && data_offset <= file_size)
    }

    pub fn is_resource_payload_path(path: &Path) -> bool {
        let lower = path.to_string_lossy().to_ascii_lowercase();
        lower.ends_with(".ress") || lower.ends_with(".resource") || lower.ends_with(".res")
    }

    pub fn normalize_memory_limit(memory_limit_bytes: u64) -> u64 {
        let max = Self::MAX_PARSE_MEMORY_LIMIT_BYTES as u64;
        if memory_limit_bytes == 0 {
            Self::auto_parse_memory_limit()
        } else {
            memory_limit_bytes.clamp(Self::MIN_PARSE_MEMORY_LIMIT_BYTES, max)
        }
    }

    pub fn estimate_parse_memory_budget(file_size: u64) -> usize {
        let file_size = file_size.min(usize::MAX as u64) as usize;
        if file_size == 0 {
            return 1;
        }
        if file_size <= Self::SMALL_BUNDLE_EXACT_MEMORY_THRESHOLD_BYTES {
            return file_size;
        }

        (file_size / 4).clamp(
            Self::LARGE_BUNDLE_MEMORY_FLOOR_BYTES,
            Self::LARGE_BUNDLE_MEMORY_CAP_BYTES,
        )
    }

    pub fn parse_worker_count(cpu_count: usize, total_files: usize) -> usize {
        if total_files == 0 {
            return 0;
        }
        cpu_count
            .saturating_mul(4)
            .max(cpu_count.saturating_add(8))
            .min(Self::MAX_PARSE_WORKER_COUNT)
            .min(total_files)
            .max(1)
    }

    pub fn result_channel_capacity(worker_count: usize) -> usize {
        worker_count.saturating_mul(16).clamp(512, 8192)
    }

    pub fn io_worker_count(cpu_count: usize, total_files: usize) -> u32 {
        if total_files == 0 {
            return 1;
        }
        cpu_count
            .saturating_mul(2)
            .clamp(8, 96)
            .min(total_files)
            .max(1) as u32
    }

    fn auto_parse_memory_limit() -> u64 {
        let mut system = sysinfo::System::new();
        system.refresh_memory();
        let total_memory = system.total_memory();
        if total_memory == 0 {
            return Self::DEFAULT_PARSE_MEMORY_LIMIT_BYTES as u64;
        }

        let total_target = total_memory.saturating_mul(3) / 4;
        let available_memory = system.available_memory();
        let available_target = if available_memory == 0 {
            total_target
        } else {
            available_memory.saturating_mul(9) / 10
        };

        total_target.min(available_target).clamp(
            Self::MIN_PARSE_MEMORY_LIMIT_BYTES,
            Self::MAX_PARSE_MEMORY_LIMIT_BYTES as u64,
        )
    }

    pub fn content_fingerprint(file_size: u64, modified_ms: u64, bytes_read: usize) -> String {
        format!("stat-v1:{}:{}:{}", file_size, modified_ms, bytes_read)
    }

    pub fn interleave_large_and_small_bundles(bundles: &mut Vec<BundleFileEntry>) {
        if bundles.len() < 4 {
            return;
        }

        let mut sorted: VecDeque<BundleFileEntry> = std::mem::take(bundles).into();
        let mut interleaved = Vec::with_capacity(sorted.len());

        while let Some(large) = sorted.pop_front() {
            interleaved.push(large);

            for _ in 0..3 {
                if let Some(small) = sorted.pop_back() {
                    interleaved.push(small);
                } else {
                    *bundles = interleaved;
                    return;
                }
            }
        }

        *bundles = interleaved;
    }
}

pub struct BuildMapMemoryLimiter {
    limit_bytes: usize,
    active_bytes: Mutex<usize>,
    condvar: Condvar,
}

pub struct BuildMapMemoryPermit<'a> {
    limiter: &'a BuildMapMemoryLimiter,
    amount_bytes: usize,
    active_counter: &'a AtomicUsize,
}

impl BuildMapMemoryLimiter {
    pub fn new(limit_bytes: usize) -> Self {
        Self {
            limit_bytes: limit_bytes.max(1),
            active_bytes: Mutex::new(0),
            condvar: Condvar::new(),
        }
    }

    pub fn acquire<'a>(
        &'a self,
        amount_bytes: usize,
        active_counter: &'a AtomicUsize,
    ) -> Result<BuildMapMemoryPermit<'a>, String> {
        let amount_bytes = amount_bytes.max(1);
        let mut guard = self
            .active_bytes
            .lock()
            .map_err(|_| "Build Map memory limiter lock poisoned".to_string())?;
        while *guard > 0 && guard.saturating_add(amount_bytes) > self.limit_bytes {
            guard = self
                .condvar
                .wait(guard)
                .map_err(|_| "Build Map memory limiter wait failed".to_string())?;
        }
        *guard = guard.saturating_add(amount_bytes);
        active_counter.fetch_add(amount_bytes, Ordering::Relaxed);
        Ok(BuildMapMemoryPermit {
            limiter: self,
            amount_bytes,
            active_counter,
        })
    }
}

impl Drop for BuildMapMemoryPermit<'_> {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.limiter.active_bytes.lock() {
            *guard = guard.saturating_sub(self.amount_bytes);
            self.active_counter
                .fetch_sub(self.amount_bytes, Ordering::Relaxed);
            self.limiter.condvar.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "assetfinder_build_support_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn detects_loose_serialized_file_header_like_assetstudio() {
        let path = temp_path("serialized-file");
        let mut bytes = vec![0u8; 24];
        bytes[0..4].copy_from_slice(&4u32.to_be_bytes()); // metadata size
        bytes[4..8].copy_from_slice(&(24u32).to_be_bytes()); // file size
        bytes[8..12].copy_from_slice(&15u32.to_be_bytes()); // version
        bytes[12..16].copy_from_slice(&20u32.to_be_bytes()); // data offset
        fs::write(&path, &bytes).unwrap();

        assert!(AssetMapBuildSupport::looks_like_serialized_file(
            &path.to_string_lossy(),
            bytes.len() as u64
        )
        .unwrap());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn detects_resource_payload_paths_for_build_map_filtering() {
        assert!(AssetMapBuildSupport::is_resource_payload_path(Path::new(
            "CAB-123.resS"
        )));
        assert!(AssetMapBuildSupport::is_resource_payload_path(Path::new(
            "foo.resource"
        )));
        assert!(AssetMapBuildSupport::is_resource_payload_path(Path::new(
            "foo.res"
        )));
        assert!(!AssetMapBuildSupport::is_resource_payload_path(Path::new(
            "bundle"
        )));
    }

    #[test]
    fn explicit_memory_limit_is_clamped_to_supported_range() {
        assert_eq!(
            AssetMapBuildSupport::normalize_memory_limit(512 * 1024 * 1024),
            1024 * 1024 * 1024
        );
        assert_eq!(
            AssetMapBuildSupport::normalize_memory_limit(256 * 1024 * 1024 * 1024),
            AssetMapBuildSupport::MAX_PARSE_MEMORY_LIMIT_BYTES as u64
        );
    }

    #[test]
    fn parse_memory_budget_uses_smaller_estimate_for_large_bundles() {
        assert_eq!(AssetMapBuildSupport::estimate_parse_memory_budget(0), 1);
        assert_eq!(
            AssetMapBuildSupport::estimate_parse_memory_budget(16 * 1024 * 1024),
            16 * 1024 * 1024
        );
        assert_eq!(
            AssetMapBuildSupport::estimate_parse_memory_budget(256 * 1024 * 1024),
            64 * 1024 * 1024
        );
        assert_eq!(
            AssetMapBuildSupport::estimate_parse_memory_budget(4 * 1024 * 1024 * 1024),
            512 * 1024 * 1024
        );
    }

    #[test]
    fn parse_workers_and_result_queue_scale_with_cpu_count() {
        assert_eq!(AssetMapBuildSupport::parse_worker_count(16, 10), 10);
        assert_eq!(AssetMapBuildSupport::parse_worker_count(16, 10_000), 64);
        assert_eq!(AssetMapBuildSupport::parse_worker_count(128, 10_000), 384);
        assert_eq!(AssetMapBuildSupport::result_channel_capacity(8), 512);
        assert_eq!(AssetMapBuildSupport::result_channel_capacity(96), 1536);
        assert_eq!(AssetMapBuildSupport::result_channel_capacity(1024), 8192);
        assert_eq!(AssetMapBuildSupport::io_worker_count(2, 1_000), 8);
        assert_eq!(AssetMapBuildSupport::io_worker_count(16, 1_000), 32);
        assert_eq!(AssetMapBuildSupport::io_worker_count(96, 1_000), 96);
        assert_eq!(AssetMapBuildSupport::io_worker_count(16, 3), 3);
    }
}
