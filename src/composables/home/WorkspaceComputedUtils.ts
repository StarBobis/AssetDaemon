/**
 * WorkspaceComputedUtils - computed value factory for the WorkSpace composable.
 *
 * The factory keeps list grouping, filtering, sorting, and preview type computation in one
 * readable utility class while useWorkspace remains the owner of the reactive state.
 */

import { computed, type Ref } from 'vue'
import type { AdvancedAssetFilterState, AssetSummary, BundleFileEntry, BundleMeta } from '../../types'
import { AssetDisplayUtils } from '../../utils/AssetDisplayUtils'

/** Computed values consumed by the WorkSpace page and child panels. */
export class WorkspaceComputedUtils {
  /** Create every computed ref used by the workspace module. */
  static create(options: {
    bundleFiles: Ref<BundleFileEntry[]>
    bundleMeta: Ref<BundleMeta | null>
    previewTargetAsset: Ref<AssetSummary | null>
    highlightedFilePaths: Ref<string[]>
    activeClassFilter: Ref<string>
    activeAssetTypeFilters?: Ref<string[]>
    assetSearchQuery: Ref<string>
    assetAdvancedFilter?: Ref<AdvancedAssetFilterState>
    collapsedGroups?: Ref<Set<string>>
    activeFilters: Ref<string[]>
    fileSizeSortDirection: Ref<'none' | 'asc' | 'desc'>
    visibleCount: Ref<number>
    visibleAssetCount: Ref<number>
    isBundleAssetMapPaged?: Ref<boolean>
    bundleAssetMapTotal?: Ref<number>
    allAssetTypes: string[]
  }) {
    const sortedFiles = computed(() => {
      const orderedFiles = options.activeFilters.value.length === 0
        ? [...options.bundleFiles.value]
        : (() => {
          const fileMap = new Map(options.bundleFiles.value.map(file => [file.path, file]))
          return options.highlightedFilePaths.value
            .map(path => fileMap.get(path))
            .filter((file): file is BundleFileEntry => file !== undefined)
        })()

      if (options.fileSizeSortDirection.value === 'none') return orderedFiles

      const direction = options.fileSizeSortDirection.value === 'asc' ? 1 : -1
      return orderedFiles.sort((left, right) => {
        const sizeDelta = (left.size - right.size) * direction
        if (sizeDelta !== 0) return sizeDelta
        return left.name.localeCompare(right.name)
      })
    })

    const visibleFiles = computed(() => sortedFiles.value.slice(0, options.visibleCount.value))
    const hasMoreFiles = computed(() => options.visibleCount.value < sortedFiles.value.length)
    const allClassNames = computed(() => options.allAssetTypes)

    const groupedAssets = computed(() => {
      const groups: Record<string, AssetSummary[]> = {}
      for (const asset of options.bundleMeta.value?.assets || []) {
        if (!groups[asset.class_name]) groups[asset.class_name] = []
        groups[asset.class_name].push(asset)
      }
      return groups
    })

    const filteredAssets = computed(() => {
      let list = options.bundleMeta.value?.assets || []
      const advancedFilter = options.assetAdvancedFilter?.value
      const activeAssetTypeFilters = advancedFilter?.classNames?.length
        ? advancedFilter.classNames
        : options.activeAssetTypeFilters?.value || []
      if (activeAssetTypeFilters.length > 0) {
        const allowedTypes = new Set(activeAssetTypeFilters)
        list = list.filter(asset => allowedTypes.has(asset.class_name))
      }
      const query = options.assetSearchQuery.value.trim().toLowerCase()
      if (query) {
        list = list.filter(asset =>
          AssetDisplayUtils.getAssetDisplayName(asset).toLowerCase().includes(query) ||
          asset.path.toLowerCase().includes(query) ||
          asset.class_name.toLowerCase().includes(query)
        )
      }
      if (advancedFilter) {
        const includeQueries = WorkspaceComputedUtils.normalizedTextList([
          advancedFilter.nameQuery,
          ...advancedFilter.nameIncludeQueries,
        ])
        const excludeQueries = WorkspaceComputedUtils.normalizedTextList([
          advancedFilter.nameExcludeQuery,
          ...advancedFilter.nameExcludeQueries,
        ])
        const minSize = WorkspaceComputedUtils.parseSizeBytes(advancedFilter.minSizeText, advancedFilter.sizeUnit)
        const maxSize = WorkspaceComputedUtils.parseSizeBytes(advancedFilter.maxSizeText, advancedFilter.sizeUnit)
        const pathIdQuery = advancedFilter.pathIdQuery.trim().toLowerCase()
        list = list.filter(asset => {
          const text = `${AssetDisplayUtils.getAssetDisplayName(asset)}\n${asset.name}\n${asset.path}\n${asset.class_name}`.toLowerCase()
          const name = AssetDisplayUtils.getAssetDisplayName(asset).toLowerCase()
          if (includeQueries.some(term => !WorkspaceComputedUtils.matchesTextMode(text, name, term, advancedFilter.nameMatchMode))) return false
          if (excludeQueries.some(term => text.includes(term.toLowerCase()))) return false
          if (pathIdQuery && !asset.path_id.toLowerCase().includes(pathIdQuery)) return false
          if (minSize !== null && asset.byte_size < minSize) return false
          if (maxSize !== null && asset.byte_size > maxSize) return false
          return true
        })

        const direction = advancedFilter.sortDirection === 'desc' ? -1 : 1
        list = [...list].sort((left, right) => {
          const delta = WorkspaceComputedUtils.compareAssets(left, right, advancedFilter.sortBy) * direction
          if (delta !== 0) return delta
          return AssetDisplayUtils.getAssetDisplayName(left).localeCompare(AssetDisplayUtils.getAssetDisplayName(right))
        })
      }
      return list
    })

    const visibleAssets = computed(() => {
      if (!options.collapsedGroups) return filteredAssets.value.slice(0, options.visibleAssetCount.value)
      const visible: AssetSummary[] = []
      for (const className of filteredAssetClassNames.value) {
        if (options.collapsedGroups.value.has(className)) continue
        const group = allFilteredGroupedAssets.value[className] || []
        for (const asset of group) {
          if (visible.length >= options.visibleAssetCount.value) return visible
          visible.push(asset)
        }
      }
      return visible
    })
    const hasMoreAssets = computed(() => options.visibleAssetCount.value < filteredAssets.value.length)

    const allFilteredGroupedAssets = computed(() => {
      const groups: Record<string, AssetSummary[]> = {}
      for (const asset of filteredAssets.value) {
        if (!groups[asset.class_name]) groups[asset.class_name] = []
        groups[asset.class_name].push(asset)
      }
      return groups
    })

    const filteredGroupedAssets = computed(() => {
      const groups: Record<string, AssetSummary[]> = {}
      let remaining = options.visibleAssetCount.value
      for (const className of filteredAssetClassNames.value) {
        const group = allFilteredGroupedAssets.value[className] || []
        if (options.collapsedGroups?.value.has(className)) {
          groups[className] = []
          continue
        }
        const visibleGroup = remaining > 0 ? group.slice(0, remaining) : []
        groups[className] = visibleGroup
        remaining -= visibleGroup.length
      }
      return groups
    })

    const filteredAssetGroupCounts = computed(() => {
      const counts: Record<string, number> = {}
      for (const asset of filteredAssets.value) {
        counts[asset.class_name] = (counts[asset.class_name] || 0) + 1
      }
      return counts
    })

    const filteredAssetClassNames = computed(() => {
      const names = Object.keys(allFilteredGroupedAssets.value)
      const primaryFilters = options.activeAssetTypeFilters?.value.length
        ? options.activeAssetTypeFilters.value
        : options.activeFilters.value
      return WorkspaceComputedUtils.sortClassNames(names, primaryFilters)
    })

    const assetClassNames = computed(() => {
      return WorkspaceComputedUtils.sortClassNames(Object.keys(groupedAssets.value), options.activeFilters.value)
    })

    const previewType = computed<'mesh' | 'texture' | 'typed' | 'none'>(() => {
      if (!options.previewTargetAsset.value) return 'none'
      switch (options.previewTargetAsset.value.class_name) {
        case 'Mesh':
        case 'Animator':
        case 'GameObject':
          return 'mesh'
        case 'Texture2D':
        case 'Sprite':
        case 'SpriteMask':
          return 'texture'
        default:
          return 'typed'
      }
    })

    return {
      sortedFiles,
      visibleFiles,
      hasMoreFiles,
      allClassNames,
      groupedAssets,
      filteredAssets,
      visibleAssets,
      hasMoreAssets,
      filteredGroupedAssets,
      filteredAssetGroupCounts,
      filteredAssetClassNames,
      assetClassNames,
      previewType,
    }
  }

  /** Sort class names with active filters first, then alphabetically. */
  private static sortClassNames(names: string[], filters: string[]): string[] {
    const sortedNames = [...names]
    if (filters.length === 0) {
      return sortedNames.sort()
    }
    const priority = new Map(filters.map((name, index) => [name, index]))
    return sortedNames.sort((left, right) => {
      const leftIndex = priority.get(left)
      const rightIndex = priority.get(right)
      if (leftIndex !== undefined && rightIndex !== undefined) return leftIndex - rightIndex
      if (leftIndex !== undefined) return -1
      if (rightIndex !== undefined) return 1
      return left.localeCompare(right)
    })
  }

  private static normalizedTextList(values: Array<string | undefined>): string[] {
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

  private static parseSizeBytes(value: string, unit: 'B' | 'KB' | 'MB'): number | null {
    const trimmed = value.trim()
    if (!trimmed) return null
    const parsed = Number(trimmed)
    if (!Number.isFinite(parsed) || parsed < 0) return null
    const multiplier = unit === 'MB' ? 1024 * 1024 : unit === 'KB' ? 1024 : 1
    return Math.floor(parsed * multiplier)
  }

  private static matchesTextMode(source: string, name: string, query: string, mode: string): boolean {
    const needle = query.toLowerCase()
    if (mode === 'starts_with') return name.startsWith(needle)
    if (mode === 'ends_with') return name.endsWith(needle)
    if (mode === 'equals') return name === needle
    return source.includes(needle)
  }

  private static compareAssets(left: AssetSummary, right: AssetSummary, sortBy: string): number {
    if (sortBy === 'size') return left.byte_size - right.byte_size
    if (sortBy === 'path_id') return WorkspaceComputedUtils.comparePathIds(left.path_id, right.path_id)
    if (sortBy === 'bundle') return (left.source_bundle_path || '').localeCompare(right.source_bundle_path || '')
    if (sortBy === 'name') return AssetDisplayUtils.getAssetDisplayName(left).localeCompare(AssetDisplayUtils.getAssetDisplayName(right))
    return left.class_name.localeCompare(right.class_name)
  }

  /** Compare Unity i64 path ids serialized as strings without losing precision. */
  private static comparePathIds(left: string, right: string): number {
    try {
      const a = BigInt(left)
      const b = BigInt(right)
      if (a < b) return -1
      if (a > b) return 1
      return 0
    } catch {
      return left.localeCompare(right)
    }
  }
}
