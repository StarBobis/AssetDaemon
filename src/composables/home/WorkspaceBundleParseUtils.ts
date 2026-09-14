/**
 * WorkspaceBundleParseUtils - Bundle parse channel result handling.
 *
 * This utility keeps verbose parse diagnostics formatting out of useWorkspace while the
 * composable still owns the actual refs and cache mutation callbacks.
 */

import { nextTick, type Ref } from 'vue'
import type { BundleMeta, BundleParseChannelResult } from '../../types'

/** Log entry accepted by useLogSystem.addLogBatch. */
type BatchLogEntry = { message: string; type?: 'info' | 'success' | 'warn' | 'error' }

/** Helper for applying parse results and formatting diagnostics. */
export class WorkspaceBundleParseUtils {
  /** Apply one async parse channel message to UI state, cache, and logs. */
  static handleParseResult(options: {
    fileName: string
    filePath: string
    message: BundleParseChannelResult
    bundleMeta: Ref<BundleMeta | null>
    isLoading: Ref<boolean>
    loadingText: Ref<string>
    setCachedBundleMeta: (path: string, meta: BundleMeta) => void
    getBundleCacheStats: () => { entries: number; usedGB: string; maxGB: string }
    t?: (key: string, named?: Record<string, unknown>) => string
    addLog: (message: string, type?: 'info' | 'success' | 'warn' | 'error') => void
    addLogBatch: (entries: BatchLogEntry[]) => void
  }): void {
    const { fileName, filePath, message, bundleMeta, isLoading, loadingText, setCachedBundleMeta, getBundleCacheStats, addLog, addLogBatch } = options
    const t = options.t || ((key: string) => key)

    if (message.success && message.meta) {
      const meta = message.meta
      bundleMeta.value = meta
      addLog(t('workspaceLog.parseComplete', { name: fileName, assets: meta.assets.length }), 'success')
      setCachedBundleMeta(filePath, meta)
      const stats = getBundleCacheStats()
      addLog(t('workspaceLog.cachedBundle', { entries: stats.entries, usedGB: stats.usedGB, maxGB: stats.maxGB }), 'info')
      WorkspaceBundleParseUtils.deferDiagnostics(meta, addLogBatch, t)
    } else {
      addLog(t('workspaceLog.parseFailed', { name: fileName, error: message.error || t('workspaceLog.unknownError') }), 'error')
    }

    isLoading.value = false
    loadingText.value = ''
  }

  /** Defer verbose diagnostics until the first parsed data frame has rendered. */
  private static deferDiagnostics(
    meta: BundleMeta,
    addLogBatch: (entries: BatchLogEntry[]) => void,
    t: (key: string, named?: Record<string, unknown>) => string,
  ): void {
    nextTick().then(() => new Promise(requestAnimationFrame)).then(() => {
      const parseLogBatch = WorkspaceBundleParseUtils.buildParseLogBatch(meta, t)
      if (parseLogBatch.length > 0) addLogBatch(parseLogBatch)

      const failedFileBatch = WorkspaceBundleParseUtils.buildFailedFileBatch(meta, t)
      if (failedFileBatch.length > 0) addLogBatch(failedFileBatch)
    })
  }

  /** Build capped parser diagnostic log entries. */
  private static buildParseLogBatch(meta: BundleMeta, t: (key: string, named?: Record<string, unknown>) => string): BatchLogEntry[] {
    const parseLogs = meta.parse_logs || []
    if (parseLogs.length === 0) return []

    const maxDiagnosticLogs = 200
    const displayLogs = parseLogs.length > maxDiagnosticLogs
      ? parseLogs.slice(0, maxDiagnosticLogs)
      : parseLogs

    const batch = displayLogs.map(logLine => ({
      message: logLine,
      type: WorkspaceBundleParseUtils.classifyParseLog(logLine),
    }))

    if (parseLogs.length > maxDiagnosticLogs) {
      batch.push({ message: t('workspaceLog.omittedDiagnostics', { count: parseLogs.length - maxDiagnosticLogs }), type: 'info' })
    }

    return batch
  }

  /** Build capped file-level diagnostic warning entries. */
  private static buildFailedFileBatch(meta: BundleMeta, t: (key: string, named?: Record<string, unknown>) => string): BatchLogEntry[] {
    const failedFiles = (meta.files_diag || []).filter(diagnostic => !diagnostic.parse_ok)
    if (failedFiles.length === 0) return []

    const batch: BatchLogEntry[] = [{ message: t('workspaceLog.nonSerializedFiles', { count: failedFiles.length }), type: 'warn' }]
    for (const diagnostic of failedFiles.slice(0, 20)) {
      batch.push({
        message: `  ${diagnostic.name} (${diagnostic.size_bytes}B, first 16 bytes: ${diagnostic.first_bytes_hex})`,
        type: 'warn',
      })
    }
    if (failedFiles.length > 20) {
      batch.push({ message: t('workspaceLog.moreFailedFiles', { count: failedFiles.length - 20 }), type: 'warn' })
    }
    return batch
  }

  /** Infer log level from backend diagnostic text. */
  private static classifyParseLog(logLine: string): BatchLogEntry['type'] {
    if (logLine.includes('FAIL') || logLine.includes('Error') || logLine.includes('Failed')) return 'error'
    if (logLine.includes('OK') || logLine.includes('Success') || logLine.includes('Completed')) return 'success'
    return 'info'
  }
}
