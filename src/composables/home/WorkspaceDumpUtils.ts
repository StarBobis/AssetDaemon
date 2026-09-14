/**
 * WorkspaceDumpUtils - TypeTree dump extraction helper.
 *
 * Keeps dump IPC and cache handling outside useWorkspace while using refs owned by the
 * composable as the single source of truth.
 */

import { type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { AssetSummary, DumpResult } from '../../types'

/** Dump helper class. */
export class WorkspaceDumpUtils {
  /** Fetch dump data with cache lookup and concurrent-load protection. */
  static async fetchDumpData(options: {
    dumpTargetAsset: Ref<AssetSummary | null>
    dumpLoading: Ref<boolean>
    dumpError: Ref<string>
    dumpData: Ref<DumpResult | null>
    dumpCache: Map<string, DumpResult | null>
    findBundleForAsset: (asset: AssetSummary) => string | null
    t?: (key: string, named?: Record<string, unknown>) => string
    addLog: (message: string, type?: 'info' | 'success' | 'warn' | 'error') => void
  }): Promise<void> {
    const { dumpTargetAsset, dumpLoading, dumpError, dumpData, dumpCache, findBundleForAsset, addLog } = options
    const t = options.t || ((key: string) => key)
    if (dumpLoading.value) return

    const asset = dumpTargetAsset.value
    if (!asset) return

    const bundlePath = findBundleForAsset(asset)
    if (!bundlePath) {
      dumpError.value = t('workspaceLog.cannotFindBundleForDump')
      dumpLoading.value = false
      return
    }

    const cacheKey = `${bundlePath}::${asset.path_id}`
    const cached = dumpCache.get(cacheKey)
    if (cached !== undefined) {
      dumpData.value = cached
      dumpError.value = ''
      dumpLoading.value = false
      return
    }

    dumpLoading.value = true
    dumpError.value = ''
    addLog(t('workspaceLog.extractingDump', { name: asset.path }), 'info')

    try {
      const result = await invoke<DumpResult>('get_asset_dump', {
        bundlePath,
        pathId: asset.path_id,
      })
      dumpCache.set(cacheKey, result)
      dumpData.value = result
      addLog(`  -> ${t('workspaceLog.dumpComplete', { hasTypeTree: result.has_typetree ? t('common.yes') : t('common.no') })}`, result.has_typetree ? 'success' : 'info')
    } catch (error) {
      dumpCache.set(cacheKey, null)
      dumpError.value = String(error)
      addLog(`  -> ${t('workspaceLog.dumpFailed', { error })}`, 'error')
    } finally {
      dumpLoading.value = false
    }
  }
}
