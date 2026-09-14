/**
 * WorkspaceWatchUtils - watcher registration for useWorkspace.
 *
 * Watchers are grouped here to keep useWorkspace readable while preserving ownership of
 * every reactive ref in the composable.
 */

import { watch, type ComputedRef, type Ref } from 'vue'
import type { AdvancedAssetFilterState, AssetSummary, BundleMeta, TypeCacheEntry } from '../../types'

/** State needed to register WorkSpace watchers. */
export interface WorkspaceWatchState {
  activeFilters: Ref<string[]>
  highlightedFilePaths: Ref<string[]>
  isFilterLoading: Ref<boolean>
  resolvedFilterTypes: Ref<Set<string>>
  fileTypeMap: Ref<Record<string, Set<string>>>
  bundleMeta: Ref<BundleMeta | null>
  groupedAssets: ComputedRef<Record<string, AssetSummary[]>>
  collapsedGroups: Ref<Set<string>>
  activeClassFilter: Ref<string>
  activeAssetTypeFilters: Ref<string[]>
  assetSearchQuery: Ref<string>
  assetAdvancedFilter: Ref<AdvancedAssetFilterState>
  persistedTypeCacheRef: { value: Record<string, TypeCacheEntry> }
}

/** Watch callback helpers supplied by useWorkspace. */
export interface WorkspaceWatchActions {
  resetFileListChunk: () => void
  resetAssetListChunk: () => void
  applyFilters: (filters: string[]) => void
  resolveFiltersFromMapOrScan: (filters: string[], options?: { allowScanFallback?: boolean }) => void
  consumeFilterScanFallback: () => boolean
  persistActiveFilters: () => Promise<void>
  t?: (key: string, named?: Record<string, unknown>) => string
  addLog: (message: string, type?: 'info' | 'success' | 'warn' | 'error') => void
}

/** Watcher utility class. */
export class WorkspaceWatchUtils {
  /** Register all module-level watchers used by the WorkSpace composable. */
  static register(state: WorkspaceWatchState, actions: WorkspaceWatchActions): void {
    WorkspaceWatchUtils.watchActiveFilters(state, actions)
    WorkspaceWatchUtils.watchFilterPersistence(state, actions)
    WorkspaceWatchUtils.watchFileTypeMap(state, actions)
    WorkspaceWatchUtils.watchBundleMeta(state)
    WorkspaceWatchUtils.watchAssetFilters(state, actions)
  }

  /** Watch selected class filters and resolve matching bundle files. */
  private static watchActiveFilters(state: WorkspaceWatchState, actions: WorkspaceWatchActions): void {
    watch(state.activeFilters, (newFilters) => {
      const allowScanFallback = actions.consumeFilterScanFallback()
      if (newFilters.length === 0) {
        state.highlightedFilePaths.value = []
        state.isFilterLoading.value = false
        actions.resetFileListChunk()
        return
      }

      if (newFilters.every(filter => state.resolvedFilterTypes.value.has(filter))) {
        actions.applyFilters(newFilters)
        return
      }

      state.isFilterLoading.value = true
      state.highlightedFilePaths.value = []
      setTimeout(() => {
        actions.addLog((actions.t || ((key: string) => key))('typeScanLog.selectedResolving', { count: newFilters.length }), 'info')
      }, 0)
      actions.resolveFiltersFromMapOrScan(newFilters, { allowScanFallback })
    }, { deep: true })
  }

  /** Persist active filter selection with debounce. */
  private static watchFilterPersistence(state: WorkspaceWatchState, actions: WorkspaceWatchActions): void {
    let saveFilterTimer: ReturnType<typeof setTimeout> | null = null
    watch(state.activeFilters, () => {
      if (saveFilterTimer) clearTimeout(saveFilterTimer)
      saveFilterTimer = setTimeout(() => {
        actions.persistActiveFilters()
      }, 500)
    }, { deep: true })
  }

  /** Apply filters as soon as file type map data changes. */
  private static watchFileTypeMap(state: WorkspaceWatchState, actions: WorkspaceWatchActions): void {
    watch(state.fileTypeMap, () => {
      if (state.activeFilters.value.length > 0) {
        actions.applyFilters(state.activeFilters.value)
        state.isFilterLoading.value = false
      }
    })
  }

  /** Collapse asset groups on bundle switch while keeping active filter groups expanded. */
  private static watchBundleMeta(state: WorkspaceWatchState): void {
    watch(state.bundleMeta, (meta) => {
      if (meta && meta.assets.length > 0) {
        const allGroups = new Set(Object.keys(state.groupedAssets.value))
        for (const filter of state.activeFilters.value) {
          if (allGroups.has(filter)) allGroups.delete(filter)
        }
        state.collapsedGroups.value = allGroups
      }
    })
  }

  /** Reset asset chunking when user changes bundle-local filters/search. */
  private static watchAssetFilters(state: WorkspaceWatchState, actions: WorkspaceWatchActions): void {
    watch([state.activeClassFilter, state.activeAssetTypeFilters, state.assetSearchQuery, state.assetAdvancedFilter], () => {
      actions.resetAssetListChunk()
    }, { deep: true })
  }

}
