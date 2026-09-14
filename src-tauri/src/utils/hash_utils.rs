/*
 * Hash utility class -- provides MD5 and other file digest computation capabilities.
 *
 * Follows Soul.md convention: struct + impl organization, no free functions.
 */

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{fs, io::Read};

/**
 * Hash computation utility class.
 *
 * Includes streaming computation of file MD5 digests and other general methods.
 * Does not involve any business logic; purely utility methods.
 */
pub struct HashUtils;

impl HashUtils {
    /**
     * Compute the MD5 digest of a file (streaming read, memory-friendly).
     *
     * Reads in 64KB buffer chunks to avoid large files consuming too much memory.
     */
    pub fn file_md5_cancelable(
        path: &str,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<String, String> {
        use md5::Digest;
        let mut file =
            fs::File::open(path).map_err(|e| format!("Unable to open file '{}': {}", path, e))?;
        let mut hasher = md5::Md5::new();
        let mut buffer = [0u8; 65536];
        loop {
            if cancel_token.is_some_and(|token| token.load(Ordering::SeqCst)) {
                return Err("Task cancelled".to_string());
            }
            let bytes_read = file
                .read(&mut buffer)
                .map_err(|e| format!("Failed to read file '{}': {}", path, e))?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }
        let digest = hasher.finalize();
        Ok(format!("{:032x}", digest))
    }
}
