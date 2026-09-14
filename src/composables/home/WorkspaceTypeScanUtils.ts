/**
 * WorkspaceTypeScanUtils - bundle type scan and filter helper.
 *
 * It centralizes the incremental MD5 cache flow and AssetMap lookup flow, keeping
 * useWorkspace smaller while preserving the same reactive refs as the source of truth.
 */

import { type Ref } from 'vue'
import { invoke, Channel } from '@tauri-apps/api/core'
import type { BundleFileEntry, TypeCacheEntry } from '../../types'
import { AssetMapCacheUtils } from '../../utils/AssetMapCacheUtils'

/** State refs used by type scanning and file filtering. */
export interface WorkspaceTypeScanState {
  workDir: Ref<string>
  bundleFiles: Ref<BundleFileEntry[]>
  fileTypeMap: Ref<Record<string, Set<string>>>
  resolvedFilterTypes: Ref<Set<string>>
  highlightedFilePaths: Ref<string[]>
  scanConcurrency: Ref<number>
  isTypeScanning: Ref<boolean>
}

/** Result of a Rust-side batch type scan. */
interface BatchScanTypeResult {
  path: string
  md5: string
  types: string[]
}

/** Type scan helper with only static class methods. */
export class WorkspaceTypeScanUtils {
  /** Scan all bundle files incrementally by comparing saved MD5 cache entries. */
  static async refreshFileTypeMap(options: {
    forceReScan: boolean
    scanningRef: { value: boolean }
    persistedTypeCache: Record<string, TypeCacheEntry>
    state: WorkspaceTypeScanState
    persistTypeCache: () => Promise<void>
    t?: (key: string, named?: Record<string, unknown>) => string
    addLog: (message: string, type?: 'info' | 'success' | 'warn' | 'error') => void
  }): Promise<void> {
    const { forceReScan, scanningRef, persistedTypeCache, state, persistTypeCache, addLog } = options
    const t = options.t || ((key: string) => key)
    if (scanningRef.value) return

    const files = state.bundleFiles.value
    if (files.length === 0) return

    scanningRef.value = true
    state.isTypeScanning.value = true

    const cached: Record<string, string> = {}
    if (!forceReScan) {
      for (const file of files) {
        const entry = persistedTypeCache[file.path]
        if (entry?.md5) {
          cached[file.path] = entry.md5
        }
      }
    }

    const label = t(forceReScan ? 'typeScanLog.forcedScan' : 'typeScanLog.incrementalScan')
    addLog(t('typeScanLog.scanStart', { label, count: files.length, concurrency: state.scanConcurrency.value }), 'info')

    const progressChannel = new Channel<{ done: number; total: number; current: string }>()
    progressChannel.onmessage = (message) => {
      addLog(`  [${message.done}/${message.total}] ${message.current}`, 'info')
    }

    try {
      const results = await invoke<BatchScanTypeResult[]>('batch_scan_types', {
        paths: files.map(file => file.path),
        cached,
        concurrency: state.scanConcurrency.value,
        onProgress: progressChannel,
      })

      const parsedCount = WorkspaceTypeScanUtils.applyScanResults(
        results,
        files,
        cached,
        persistedTypeCache,
        state.fileTypeMap,
        state.resolvedFilterTypes,
      )

      await persistTypeCache()

      if (forceReScan) {
        addLog(t('typeScanLog.forcedComplete', { count: parsedCount }), 'success')
      } else {
        addLog(t('typeScanLog.incrementalComplete', { parsed: parsedCount, cached: files.length - parsedCount }), 'success')
      }
    } catch (error) {
      addLog(t('typeScanLog.failed', { error }), 'error')
    } finally {
      scanningRef.value = false
      state.isTypeScanning.value = false
    }
  }

  /** Resolve filters from the AssetMap index first, then tell caller whether scan fallback is needed. */
  static async resolveFiltersFromMap(options: {
    filters: string[]
    state: WorkspaceTypeScanState
    t?: (key: string, named?: Record<string, unknown>) => string
    addLog: (message: string, type?: 'info' | 'success' | 'warn' | 'error') => void
  }): Promise<boolean> {
    const { filters, state, addLog } = options
    const t = options.t || ((key: string) => key)
    if (!state.workDir.value) return false

    try {
      const assetMapCacheRoot = await AssetMapCacheUtils.getCacheRoot()
      const result = await invoke<Record<string, string[]>>('get_map_bundle_paths_by_classes', {
        workspaceDir: state.workDir.value,
        classNames: filters,
        assetMapCacheRoot,
      })
      const nextMap: Record<string, Set<string>> = { ...state.fileTypeMap.value }
      for (const [className, paths] of Object.entries(result)) {
        state.resolvedFilterTypes.value.add(className)
        for (const path of paths) {
          if (!nextMap[path]) nextMap[path] = new Set()
          nextMap[path].add(className)
        }
      }
      state.fileTypeMap.value = nextMap
      addLog(t('typeScanLog.resolvedFromMap', { filters: filters.join(', ') }), 'success')
      return true
    } catch (error) {
      addLog(t('typeScanLog.mapFallback', { error }), 'warn')
      return false
    }
  }

  /** Apply selected filters and return how many files matched. */
  static applyFilters(options: {
    filters: string[]
    bundleFiles: BundleFileEntry[]
    fileTypeMap: Record<string, Set<string>>
    highlightedFilePaths: Ref<string[]>
  }): number {
    const { filters, bundleFiles, fileTypeMap, highlightedFilePaths } = options
    if (filters.length === 0 || bundleFiles.length === 0) {
      highlightedFilePaths.value = []
      return 0
    }

    const matched: string[] = []
    for (const file of bundleFiles) {
      const types = fileTypeMap[file.path]
      if (types && filters.some(filter => types.has(filter))) {
        matched.push(file.path)
      }
    }

    if (matched.length === 0) {
      if (highlightedFilePaths.value.length > 0) {
        highlightedFilePaths.value = []
      }
      return 0
    }

    highlightedFilePaths.value = matched
    return matched.length
  }

  /** Merge Rust scan results into reactive map and persistent cache. */
  private static applyScanResults(
    results: BatchScanTypeResult[],
    files: BundleFileEntry[],
    cached: Record<string, string>,
    persistedTypeCache: Record<string, TypeCacheEntry>,
    fileTypeMap: Ref<Record<string, Set<string>>>,
    resolvedFilterTypes: Ref<Set<string>>,
  ): number {
    const newMap: Record<string, Set<string>> = {}
    let parsedCount = 0

    for (const result of results) {
      if (result.types.length > 0) {
        newMap[result.path] = new Set(result.types)
        persistedTypeCache[result.path] = { md5: result.md5, types: result.types }
        for (const type of result.types) {
          resolvedFilterTypes.value.add(type)
        }
        parsedCount++
      } else if (cached[result.path]) {
        const cachedEntry = persistedTypeCache[result.path]
        newMap[result.path] = new Set(cachedEntry?.types || [])
        for (const type of cachedEntry?.types || []) {
          resolvedFilterTypes.value.add(type)
        }
      }
    }

    for (const file of files) {
      if (!(file.path in newMap)) {
        const entry = persistedTypeCache[file.path]
        newMap[file.path] = new Set(entry?.types || [])
      }
    }

    fileTypeMap.value = newMap
    return parsedCount
  }
}
