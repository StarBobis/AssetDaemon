import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import type { AdvancedAssetFilterState, AssetSummary, BundleMeta, MapAssetClassStat, MapAssetQueryResult, MapSummary } from '../../types'
import { ASSET_MAP_FULL_QUERY_LIMIT } from './WorkspaceAssetMapState'
import { AssetMapCacheUtils } from '../../utils/AssetMapCacheUtils'

type InvokeFn = <T>(command: string, args?: Record<string, unknown>) => Promise<T>

export interface BundleAssetMapPageRequest {
  workspaceDir: string
  bundlePath: string
  search: string
  classNames: string[]
  advancedFilter?: AdvancedAssetFilterState
  offset: number
  limit?: number
}

export interface WorkspaceAssetMapClientDeps {
  invoke: InvokeFn
  getCacheRoot: () => Promise<string>
}

/**
 * Deep interface for AssetMap-backed workspace reads.
 *
 * Callers describe the current workspace intent; this module hides the Tauri
 * command names, cache-root lookup, query shape, and empty BundleMeta scaffold.
 */
export class WorkspaceAssetMapClient {
  constructor(private readonly deps: WorkspaceAssetMapClientDeps) {}

  createPagedBundleMeta(bundlePath: string): BundleMeta {
    return {
      path: bundlePath,
      size: 0,
      compressed: false,
      unity_version: '',
      nodes: [],
      assets: [],
      files_diag: [],
      parse_logs: ['(Loaded from AssetMap index pages)'],
    }
  }

  applyBundleAssetPage(
    currentMeta: BundleMeta | null,
    bundlePath: string,
    assets: AssetSummary[],
    reset: boolean,
  ): BundleMeta {
    const currentAssets = reset ? [] : (currentMeta?.assets || [])
    return {
      ...(currentMeta || this.createPagedBundleMeta(bundlePath)),
      path: bundlePath,
      assets: [...currentAssets, ...assets],
    }
  }

  async loadBundleAssets(request: BundleAssetMapPageRequest): Promise<MapAssetQueryResult> {
    const assetMapCacheRoot = await this.deps.getCacheRoot()
    return this.deps.invoke<MapAssetQueryResult>('query_map_assets', {
      workspaceDir: request.workspaceDir,
      assetMapCacheRoot,
      options: {
        search: request.search,
        class_names: request.classNames,
        bundle_path: request.bundlePath,
        ...this.buildAdvancedOptions(request.advancedFilter),
        offset: request.offset,
        limit: request.limit ?? ASSET_MAP_FULL_QUERY_LIMIT,
      },
    })
  }

  async loadBundleAssetPage(request: BundleAssetMapPageRequest): Promise<MapAssetQueryResult> {
    return this.loadBundleAssets(request)
  }

  async hasAssetMap(workspaceDir: string): Promise<boolean> {
    const assetMapCacheRoot = await this.deps.getCacheRoot()
    const summary = await this.deps.invoke<MapSummary | null>('get_map_summary', {
      workspaceDir,
      assetMapCacheRoot,
    })
    return summary !== null
  }

  private buildAdvancedOptions(filter?: AdvancedAssetFilterState): Record<string, unknown> {
    if (!filter) return {}
    const includeQueries = this.normalizedTextList([
      filter.nameQuery,
      ...filter.nameIncludeQueries,
    ])
    const excludeQueries = this.normalizedTextList([
      filter.nameExcludeQuery,
      ...filter.nameExcludeQueries,
    ])
    return {
      name_query: filter.nameQuery.trim() || undefined,
      name_match_mode: filter.nameMatchMode,
      name_include_queries: includeQueries.length ? includeQueries : undefined,
      name_exclude_query: filter.nameExcludeQuery.trim() || undefined,
      name_exclude_queries: excludeQueries.length ? excludeQueries : undefined,
      path_id_query: filter.pathIdQuery.trim() || undefined,
      min_size: this.parseSizeBytes(filter.minSizeText, filter.sizeUnit),
      max_size: this.parseSizeBytes(filter.maxSizeText, filter.sizeUnit),
      sort_by: filter.sortBy,
      sort_direction: filter.sortDirection,
    }
  }

  private normalizedTextList(values: Array<string | undefined>): string[] {
    const seen = new Set<string>()
    const result: string[] = []
    for (const value of values) {
      const trimmed = value?.trim()
      if (!trimmed) continue
      const key = trimmed.toLocaleLowerCase()
      if (seen.has(key)) continue
      seen.add(key)
      result.push(trimmed)
    }
    return result
  }

  private parseSizeBytes(value: string, unit: 'B' | 'KB' | 'MB'): number | null {
    const trimmed = value.trim()
    if (!trimmed) return null
    const parsed = Number(trimmed)
    if (!Number.isFinite(parsed) || parsed < 0) return null
    const multiplier = unit === 'MB' ? 1024 * 1024 : unit === 'KB' ? 1024 : 1
    return Math.floor(parsed * multiplier)
  }

  async loadBundleClassStats(workspaceDir: string, bundlePath: string): Promise<MapAssetClassStat[]> {
    const assetMapCacheRoot = await this.deps.getCacheRoot()
    return this.deps.invoke<MapAssetClassStat[]>('get_map_bundle_asset_class_stats', {
      workspaceDir,
      bundlePath,
      assetMapCacheRoot,
    })
  }
}

export const workspaceAssetMapClient = new WorkspaceAssetMapClient({
  invoke: tauriInvoke as InvokeFn,
  getCacheRoot: () => AssetMapCacheUtils.getCacheRoot(),
})
