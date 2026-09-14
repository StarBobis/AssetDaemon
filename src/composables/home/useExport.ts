/**
 * useExport ->Batch Export Composable
 *
 * Manages the full lifecycle of batch export:
 *   - Dialog open/close
 *   - Asset selection and format configuration
 *   - Calls Rust start_export_batch + Channel progress
 *   - Cancellation and reporting
 *
 * Uses module-level singleton pattern, consistent with useWorkspace / useLogSystem.
 */

import { ref } from 'vue'
import { invoke, Channel } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { load } from '@tauri-apps/plugin-store'
import { revealItemInDir, openPath } from '@tauri-apps/plugin-opener'
import { i18n } from '../../i18n'
import { useLogSystem } from '../useLogSystem'
import { useWorkspace } from './useWorkspace'
import { ExportFormatUtils } from '../../utils/ExportFormatUtils'
import { AssetMapCacheUtils } from '../../utils/AssetMapCacheUtils'
import {
  type ExportAssetRef,
  type ExportFormat,
  type ExportGroupBy,
  type ExportJob,
  type ExportProgress,
  type ExportReport,
} from '../../types'

// ============================================================
// Module-level state (singleton)
// ============================================================

const showDialog = ref(false)
const isExporting = ref(false)
const report = ref<ExportReport | null>(null)

/** Assets to be exported */
const selectedAssets = ref<ExportAssetRef[]>([])

/** Output directory */
const outputDir = ref('')

/** Grouping method */
const groupBy = ref<ExportGroupBy>('none')

/** Overwrite existing files */
const overwriteExisting = ref(false)

/** Generate report */
const generateReport = ref(false)

/** Heuristic GLB export content toggles. These guard backend-side extra scans. */
const includeMaterials = ref(true)
const includeTextures = ref(true)
const includeSkeleton = ref(true)
const includeAnimations = ref(true)
const splitSubmeshes = ref(false)

/** Type ->format mapping (user-configurable) */
const formatOverrides = ref<Record<string, ExportFormat>>({})

/** Current export job ID (for cancellation) */
const currentJobId = ref('')

// ============================================================
// Export functions
// ============================================================

export function useExport() {
  const log = useLogSystem()
  const ws = useWorkspace()

  const warn = (msg: string) => log.addLog(msg, 'warn')
  const info = (msg: string) => log.addLog(msg, 'info')
  const error = (msg: string) => log.addLog(msg, 'error')
  const success = (msg: string) => log.addLog(msg, 'success')
  const t = i18n.global.t

  /** Generate unique job ID */
  function genJobId(): string {
    return `export_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`
  }

  /** Open export dialog */
  function openExportDialog(assets: ExportAssetRef[]) {
    if (assets.length === 0) {
      warn(t('exportDialog.noAssets'))
      return
    }
    selectedAssets.value = assets

    // Initialize with default recommended format for each type
    const overrides: Record<string, ExportFormat> = {}
    for (const a of assets) {
      if (!overrides[a.class_name]) {
        overrides[a.class_name] = ExportFormatUtils.getRecommendedFormat(a.class_name)
      }
    }
    formatOverrides.value = overrides
    report.value = null

    // Non-blocking load of last export preferences
    load('settings.json', { defaults: {}, autoSave: true }).then(store => {
      store.get<string>('export-output-dir').then(d => { if (d) outputDir.value = d })
      store.get<ExportGroupBy>('export-group-by').then(g => { if (g) groupBy.value = g })
      store.get<boolean>('export-overwrite-existing').then(v => { if (v != null) overwriteExisting.value = v })
      store.get<boolean>('export-generate-report').then(v => { if (v != null) generateReport.value = v })
      store.get<boolean>('export-include-materials').then(v => { if (v != null) includeMaterials.value = v })
      store.get<boolean>('export-include-textures').then(v => { if (v != null) includeTextures.value = v })
      store.get<boolean>('export-include-skeleton').then(v => { if (v != null) includeSkeleton.value = v })
      store.get<boolean>('export-include-animations').then(v => { if (v != null) includeAnimations.value = v })
      store.get<boolean>('export-split-submeshes').then(v => { if (v != null) splitSubmeshes.value = v })
      store.get<Record<string, ExportFormat>>('export-format-overrides').then(saved => {
        if (saved && typeof saved === 'object') {
          // Apply saved overrides only for types present in current selection
          const merged = { ...formatOverrides.value }
          for (const className of Object.keys(saved)) {
            if (merged[className] !== undefined) {
              merged[className] = normalizeSavedFormat(className, saved[className])
            }
          }
          formatOverrides.value = merged
        }
      })
    }).catch(() => {})

    // Show dialog immediately
    showDialog.value = true
  }

  async function exportWithDefaults(assets: ExportAssetRef[]) {
    if (assets.length === 0) {
      warn(t('exportDialog.noAssets'))
      return
    }

    selectedAssets.value = assets
    const overrides: Record<string, ExportFormat> = {}
    for (const a of assets) {
      if (!overrides[a.class_name]) {
        overrides[a.class_name] = ExportFormatUtils.getRecommendedFormat(a.class_name)
      }
    }
    formatOverrides.value = overrides

    if (!outputDir.value) {
      try {
        const store = await load('settings.json', { defaults: {}, autoSave: true })
        outputDir.value = await store.get<string>('export-output-dir') || ''
      } catch (_) { /* Missing preference falls through to folder picker */ }
    }

    if (!outputDir.value) {
      const dir = await open({ directory: true, multiple: false, title: t('exportDialog.selectDirectoryTitle') })
      if (!dir || typeof dir !== 'string') {
        return
      }
      outputDir.value = dir
    }

    await startExport()
  }

  /** Select output directory */
  async function browseOutputDir() {
    const dir = await open({ directory: true, multiple: false, title: t('exportDialog.selectDirectoryTitle') })
    if (dir && typeof dir === 'string') {
      outputDir.value = dir
    }
  }

  /** Toggle export format for a type */
  function setFormat(className: string, format: ExportFormat) {
    formatOverrides.value = { ...formatOverrides.value, [className]: format }
  }

  function normalizeSavedFormat(className: string, format: ExportFormat): ExportFormat {
    if (className === 'GameObject' && format !== 'glb') {
      return ExportFormatUtils.getRecommendedFormat(className)
    }
    return format
  }

  /** Toggle whether a type participates in export */
  function toggleType(className: string) {
    const current = formatOverrides.value[className]
    if (current) {
      // Selected ->unset (remove entry)
      const { [className]: _, ...rest } = formatOverrides.value
      formatOverrides.value = rest
    } else {
      // Not selected ->restore default
      formatOverrides.value = {
        ...formatOverrides.value,
        [className]: ExportFormatUtils.getRecommendedFormat(className),
      }
    }
  }

  /** Start export */
  async function startExport() {
    if (!outputDir.value) {
      warn(t('exportDialog.selectOutputFirst'))
      return
    }
    const activeTypes = Object.keys(formatOverrides.value)
    const assetsToExport = selectedAssets.value.filter(a => activeTypes.includes(a.class_name))
    if (assetsToExport.length === 0) {
      warn(t('exportDialog.noAssetTypeSelected'))
      return
    }

    const jobId = genJobId()
    currentJobId.value = jobId
    isExporting.value = true
    showDialog.value = false
    report.value = null
    const workspaceDirs = ws.workDir.value ? [ws.workDir.value] : []
    // getCacheRoot() is called before the try/finally below; if it rejects we must
    // reset the exporting state ourselves or the UI deadlocks (isExporting stays true).
    let assetMapCacheRoot: string
    try {
      assetMapCacheRoot = await AssetMapCacheUtils.getCacheRoot()
    } catch (e) {
      isExporting.value = false
      showDialog.value = true
      error(t('exportDialog.failed', { error: e }))
      return
    }

    const job: ExportJob = {
      job_id: jobId,
      assets: assetsToExport,
      options: {
        output_dir: outputDir.value,
        group_by: groupBy.value,
        overwrite_existing: overwriteExisting.value,
        generate_report: generateReport.value,
        include_materials: includeMaterials.value,
        include_textures: includeTextures.value,
        include_skeleton: includeSkeleton.value,
        include_animations: includeAnimations.value,
        split_submeshes: splitSubmeshes.value,
        workspace_dirs: workspaceDirs,
        asset_map_cache_root: assetMapCacheRoot,
        format_overrides: formatOverrides.value,
      },
    }

    info(t('exportDialog.starting', { count: assetsToExport.length, path: outputDir.value }))

    try {
      const channel = new Channel<ExportProgress>()
      let lastProgressLog = 0
      channel.onmessage = (p: ExportProgress) => {
        if (p.cancelled) {
          warn(t('exportDialog.cancelled'))
          return
        }
        // Throttle progress logs to avoid flooding
        const now = Date.now()
        if (p.done > 0 && p.current_asset && now - lastProgressLog >= 2000) {
          lastProgressLog = now
          info(t('exportDialog.progress', {
            done: p.done,
            total: p.total,
            asset: p.current_asset,
            step: p.current_step,
          }))
        }
      }

      const result = await invoke<ExportReport>('start_export_batch', {
        job,
        progress: channel,
      })

      report.value = result
      // Save export preferences
      try {
        const store = await load('settings.json', { defaults: {}, autoSave: true })
        await store.set('export-output-dir', outputDir.value)
        await store.set('export-group-by', groupBy.value)
        await store.set('export-overwrite-existing', overwriteExisting.value)
        await store.set('export-generate-report', generateReport.value)
        await store.set('export-include-materials', includeMaterials.value)
        await store.set('export-include-textures', includeTextures.value)
        await store.set('export-include-skeleton', includeSkeleton.value)
        await store.set('export-include-animations', includeAnimations.value)
        await store.set('export-split-submeshes', splitSubmeshes.value)
        await store.set('export-format-overrides', formatOverrides.value)
        await store.save()
      } catch (_) { /* Save failure does not affect export */ }

      // Log results of each file
      for (const r of result.results) {
        if (r.success) {
          success(t('exportDialog.itemSuccess', {
            name: r.asset_name,
            file: r.output_file.split(/[/\\]/).pop() || r.output_file,
            bytes: r.byte_size,
            ms: r.duration_ms,
          }))
        } else {
          error(t('exportDialog.itemFailed', { name: r.asset_name, error: r.error || t('workspaceLog.unknownError') }))
        }
      }
      const ts = (result.duration_ms / 1000).toFixed(1)
      success(t('exportDialog.complete', {
        succeeded: result.succeeded,
        total: result.total,
        seconds: ts,
        path: result.output_directory,
      }))

      // Open file manager and close dialog after export
      try {
        const okFiles = result.results.filter(r => r.success && r.output_file)
        if (okFiles.length === 1) {
          await revealItemInDir(okFiles[0].output_file)
        } else if (okFiles.length > 1) {
          await openPath(outputDir.value)
        }
      } catch (_) { /* File manager open failure does not affect */ }
    } catch (e) {
      error(t('exportDialog.failed', { error: e }))
      isExporting.value = false
    } finally {
      isExporting.value = false
    }
  }

  /** Cancel export */
  async function cancelExport() {
    if (!currentJobId.value) return
    try {
      await invoke('cancel_export', { jobId: currentJobId.value })
      warn(t('exportDialog.cancelRequested'))
    } catch (e) {
      error(t('exportDialog.cancelFailed', { error: e }))
    }
  }

  /** Close dialog */
  function closeDialog() {
    showDialog.value = false
  }

  /** Reset state */
  function resetExport() {
    showDialog.value = false
    isExporting.value = false
    report.value = null
    selectedAssets.value = []
    currentJobId.value = ''
  }

  return {
    // State
    showDialog,
    isExporting,
    report,
    selectedAssets,
    outputDir,
    groupBy,
    overwriteExisting,
    generateReport,
    includeMaterials,
    includeTextures,
    includeSkeleton,
    includeAnimations,
    splitSubmeshes,
    formatOverrides,
    currentJobId,
    // Computed
    // Actions
    openExportDialog,
    exportWithDefaults,
    browseOutputDir,
    setFormat,
    toggleType,
    startExport,
    cancelExport,
    closeDialog,
    resetExport,
  }
}
