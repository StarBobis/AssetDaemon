/*
 * Decrypt command -- Decrypt all .bundle files under the GF2 AssetBundle directory.
 *
 * This file only contains #[tauri::command] functions; the actual decryption logic is delegated to DecryptionService.
 * Follows Soul.md convention: separation of commands and service classes.
 */

use crate::common::task::task_logger::TaskLogger;
use crate::common::task::task_runner::TaskRunner;
use crate::decrypt::girls_frontline2::gf2::{DecryptResult as Gf2DecryptResult, DecryptionService};
use crate::decrypt::naraka_bladepoint::nbp_v11::{
    DecryptResult as NarakaDecryptResult, NarakaBladepointDecryptionService,
};
use crate::decrypt::the_magic_blade::the_magic_blade_v1::{
    DecryptResult as TheMagicBladeDecryptResult, TheMagicBladeDecryptionService,
};
/**
 * Decrypt all .bundle files under the GF2 AssetBundle directory.
 *
 * This command is called by the frontend Decrypt page. The workflow is as follows:
 * 1. Scan the input directory and recursively collect all .bundle files
 * 2. Decrypt in batches with concurrency control
 * 3. Stream progress to the frontend via Channel
 * 4. Return a summary of decryption results
 *
 * @param on_progress       Tauri IPC Channel, used to stream progress
 * @param input_path        Input directory (game AssetBundles_Windows path)
 * @param output_path       Output directory (where decrypted files are saved)
 * @param max_concurrency   Maximum concurrency (1 ~ 50, default 20)
 * @return                  DecryptResult JSON, containing total / success / failed / elapsed, etc.
 */
#[tauri::command]
pub async fn decrypt_gf2_files(
    input_path: String,
    output_path: String,
    max_concurrency: u32,
) -> Gf2DecryptResult {
    match TaskRunner::run_blocking(
        "decrypt_gf2",
        "Decrypt GF2",
        format!("Starting decrypt: {}", input_path),
        None,
        move |ctx| {
            let result = DecryptionService::decrypt_all_bundles(
                ctx.task_id(),
                &ctx.cancel_token(),
                &input_path,
                &output_path,
                max_concurrency as usize,
            );
            log_decrypt_result(ctx.task_id(), "Decrypt GF2", &result);
            Ok(result)
        },
    )
    .await
    {
        Ok(result) => result,
        Err(e) => Gf2DecryptResult {
            total: 0,
            success: 0,
            failed: 1,
            failed_files: vec![format!("Decrypt worker failed: {}", e)],
            elapsed_secs: 0.0,
            cancelled: false,
        },
    }
}

#[tauri::command]
pub async fn decrypt_naraka_bladepoint_files(
    input_path: String,
    output_path: String,
    max_concurrency: u32,
) -> NarakaDecryptResult {
    match TaskRunner::run_blocking(
        "decrypt_naraka",
        "Decrypt Naraka",
        format!("Starting decrypt: {}", input_path),
        None,
        move |ctx| {
            let result = NarakaBladepointDecryptionService::decrypt_all_bundles(
                ctx.task_id(),
                &ctx.cancel_token(),
                &input_path,
                &output_path,
                max_concurrency as usize,
            );
            log_decrypt_result(ctx.task_id(), "Decrypt Naraka", &result);
            Ok(result)
        },
    )
    .await
    {
        Ok(result) => result,
        Err(e) => NarakaDecryptResult {
            total: 0,
            success: 0,
            failed: 1,
            failed_files: vec![format!("Decrypt worker failed: {}", e)],
            elapsed_secs: 0.0,
            cancelled: false,
        },
    }
}

#[tauri::command]
pub async fn decrypt_the_magic_blade_files(
    input_path: String,
    output_path: String,
    max_concurrency: u32,
) -> TheMagicBladeDecryptResult {
    match TaskRunner::run_blocking(
        "decrypt_the_magic_blade",
        "Decrypt The Magic Blade",
        format!("Starting decrypt: {}", input_path),
        None,
        move |ctx| {
            let result = TheMagicBladeDecryptionService::decrypt_all_bundles(
                ctx.task_id(),
                &ctx.cancel_token(),
                &input_path,
                &output_path,
                max_concurrency as usize,
            );
            log_decrypt_result(ctx.task_id(), "Decrypt The Magic Blade", &result);
            Ok(result)
        },
    )
    .await
    {
        Ok(result) => result,
        Err(e) => TheMagicBladeDecryptResult {
            total: 0,
            success: 0,
            failed: 1,
            failed_files: vec![format!("Decrypt worker failed: {}", e)],
            elapsed_secs: 0.0,
            cancelled: false,
        },
    }
}

fn log_decrypt_result<T>(task_id: &str, task_label: &str, result: &T)
where
    T: DecryptSummary,
{
    if result.is_cancelled() {
        TaskLogger::warn(
            task_id,
            task_label,
            &format!(
                "Decrypt cancelled: {} processed, {} failed, {:.2}s",
                result.success_count(),
                result.failed_count(),
                result.elapsed_secs()
            ),
        );
    } else if result.failed_count() == 0 {
        TaskLogger::success(
            task_id,
            task_label,
            &format!(
                "Decrypt complete: {} succeeded, {} failed, {:.2}s",
                result.success_count(),
                result.failed_count(),
                result.elapsed_secs()
            ),
        );
    } else {
        TaskLogger::error(
            task_id,
            task_label,
            &format!(
                "Decrypt complete with errors: {} succeeded, {} failed, {:.2}s",
                result.success_count(),
                result.failed_count(),
                result.elapsed_secs()
            ),
        );
    }
}

trait DecryptSummary {
    fn success_count(&self) -> usize;
    fn failed_count(&self) -> usize;
    fn elapsed_secs(&self) -> f64;
    fn is_cancelled(&self) -> bool;
}

impl DecryptSummary for Gf2DecryptResult {
    fn success_count(&self) -> usize {
        self.success
    }

    fn failed_count(&self) -> usize {
        self.failed
    }

    fn elapsed_secs(&self) -> f64 {
        self.elapsed_secs
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}

impl DecryptSummary for NarakaDecryptResult {
    fn success_count(&self) -> usize {
        self.success
    }

    fn failed_count(&self) -> usize {
        self.failed
    }

    fn elapsed_secs(&self) -> f64 {
        self.elapsed_secs
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}

impl DecryptSummary for TheMagicBladeDecryptResult {
    fn success_count(&self) -> usize {
        self.success
    }

    fn failed_count(&self) -> usize {
        self.failed
    }

    fn elapsed_secs(&self) -> f64 {
        self.elapsed_secs
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}
