<script lang="ts">
import type {
  AdvancedAssetFilterState as CachedAdvancedAssetFilterState,
  AssetSummary as CachedAssetSummary,
  AssetSearchIndexStatus as CachedAssetSearchIndexStatus,
  MapAssetClassStat as CachedMapAssetClassStat,
  MapSummary as CachedMapSummary,
} from '../../types'

/** Cross-mount cache for the All Assets tab. */
class AllAssetsPageCache {
  static workspaceDirectory = ''
  static query = ''
  static activeTypeFilters: string[] = []
  static advancedFilter: CachedAdvancedAssetFilterState | null = null
  static mapSummary: CachedMapSummary | null = null
  static searchIndexStatus: CachedAssetSearchIndexStatus | null = null
  static typeStats: CachedMapAssetClassStat[] = []
  static assets: CachedAssetSummary[] = []
  static totalAssets = 0
  static visibleAssetCount = 300
  static collapsedGroups: string[] = []
  static loadError = ''
  static initialized = false
}
</script>

<script setup lang="ts">
/**
 * AllAssetsPanel -> AssetMap-backed global asset search panel.
 *
 * This tab is intentionally driven by the AssetMap index instead of opened Bundle cache.
 * Build Index creates the index, then this panel queries bounded pages on demand.
 */

import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Search, FolderOpened, Refresh } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import { useWorkspace } from '../../composables/home/useWorkspace'
import { useExport } from '../../composables/home/useExport'
import { WorkspaceStoreUtils } from '../../composables/home/WorkspaceStoreUtils'
import { useLogSystem } from '../../composables/useLogSystem'
import { AssetDisplayUtils } from '../../utils/AssetDisplayUtils'
import { ClassIconUtils } from '../../utils/ClassIconUtils'
import AssetContextMenu from './AssetContextMenu.vue'
import AssetListItem from './AssetListItem.vue'
import AdvancedAssetFilterCard from './AdvancedAssetFilterCard.vue'
import type { AdvancedAssetFilterState, AssetSearchIndexStatus, AssetSummary, ExportAssetRef, MapAssetClassStat, MapAssetQueryResult, MapSummary } from '../../types'
import { AssetMapCacheUtils } from '../../utils/AssetMapCacheUtils'
import { createDefaultAdvancedAssetFilter, normalizeAdvancedAssetFilter } from '../../utils/AdvancedAssetFilterUtils'
import { ASSET_MAP_FULL_QUERY_LIMIT } from '../../composables/home/WorkspaceAssetMapState'

const ws = useWorkspace()
const exp = useExport()
const logSystem = useLogSystem()
const { t } = useI18n()
const hasWarmCacheForCurrentWorkspace = AllAssetsPageCache.initialized &&
  AllAssetsPageCache.workspaceDirectory === ws.workDir.value

/** Keep Vue responsive by mounting large result sets in chunks while keeping all rows in memory. */
const ALL_ASSETS_RENDER_CHUNK_SIZE = 300
const SEARCH_INDEX_POLL_INITIAL_DELAY_MS = 1_500
const SEARCH_INDEX_POLL_INTERVAL_MS = 2_500
const SEARCH_INDEX_POLL_ERROR_DELAY_MS = 5_000

/** Search text used only for local in-memory filtering of already loaded results. */
const query = ref(hasWarmCacheForCurrentWorkspace ? AllAssetsPageCache.query : '')

/** Local multi-select class filters for the AssetMap-backed All Assets tab. */
const activeTypeFilters = ref<string[]>(hasWarmCacheForCurrentWorkspace ? [...AllAssetsPageCache.activeTypeFilters] : [])

const advancedFilterVisible = ref(false)

const advancedFilter = ref<AdvancedAssetFilterState>(
  hasWarmCacheForCurrentWorkspace && AllAssetsPageCache.advancedFilter
    ? normalizeAdvancedAssetFilter(AllAssetsPageCache.advancedFilter)
    : createDefaultAdvancedAssetFilter(),
)

/** AssetMap summary decides whether the tab can be used. */
const mapSummary = ref<MapSummary | null>(hasWarmCacheForCurrentWorkspace ? AllAssetsPageCache.mapSummary : null)

/** Optional trigram search index status for fast contains filters. */
const searchIndexStatus = ref<AssetSearchIndexStatus | null>(
  hasWarmCacheForCurrentWorkspace ? AllAssetsPageCache.searchIndexStatus : null,
)
const searchIndexBuildRequested = ref(false)

/** Type counts read from the AssetMap index for the filter popup. */
const typeStats = ref<MapAssetClassStat[]>(hasWarmCacheForCurrentWorkspace ? [...AllAssetsPageCache.typeStats] : [])

/** Full in-memory AssetMap result set; DOM rendering is chunked separately. */
const assets = ref<AssetSummary[]>(hasWarmCacheForCurrentWorkspace ? [...AllAssetsPageCache.assets] : [])

/** Total matching rows reported by the AssetMap index. */
const totalAssets = ref(hasWarmCacheForCurrentWorkspace ? AllAssetsPageCache.totalAssets : 0)
const visibleAssetCount = ref(hasWarmCacheForCurrentWorkspace ? AllAssetsPageCache.visibleAssetCount : ALL_ASSETS_RENDER_CHUNK_SIZE)
const collapsedGroups = ref<Set<string>>(
  new Set(hasWarmCacheForCurrentWorkspace ? AllAssetsPageCache.collapsedGroups : []),
)

/** Loading states for first page and incremental pages. */
const loading = ref(false)
const querying = ref(false)

/** Last backend error shown in the empty state area. */
const loadError = ref(hasWarmCacheForCurrentWorkspace ? AllAssetsPageCache.loadError : '')

/** Monotonic query token prevents stale slow queries from replacing newer results. */
let querySequence = 0
let activeQueryToken = 0
let lastStopSignal = logSystem.stopSignal.value
let searchIndexPollTimer: ReturnType<typeof window.setTimeout> | null = null
let searchIndexPollToken = 0
let unregisterAllAssetsKeyboardNavigator: (() => void) | null = null
let searchQueryTimer: ReturnType<typeof window.setTimeout> | null = null
const lastSelectionAnchorKey = ref<string | null>(null)
const selectionDrag = ref<{
  originX: number
  originY: number
  currentX: number
  currentY: number
  active: boolean
  additive: boolean
} | null>(null)
let suppressNextAssetClick = false

function setAllAssetsListElement(element: unknown) {
  ws.allAssetsListRef.value = element instanceof HTMLElement ? element : null
}

function focusAllAssetsList() {
  ws.setKeyboardNavigationScope('all-assets')
  ws.allAssetsListRef.value?.focus({ preventScroll: true })
}

function selectedRefKey(ref: ExportAssetRef): string {
  return `${ref.bundle_path}::${ref.path_id}`
}

function exportRefForAsset(asset: AssetSummary): ExportAssetRef {
  return {
    bundle_path: asset.source_bundle_path || '',
    path_id: asset.path_id,
    class_name: asset.class_name,
    asset_name: AssetDisplayUtils.getAssetExportName(asset),
  }
}

const selectedAssetKeys = computed(() => new Set(exp.selectedAssets.value.map(selectedRefKey)))

function isAssetSelected(asset: AssetSummary): boolean {
  return selectedAssetKeys.value.has(AllAssetsDisplayUtils.getAssetKey(asset))
}

function clearSelection() {
  exp.selectedAssets.value = []
  lastSelectionAnchorKey.value = null
}

function setSelectedAssetRefs(refs: ExportAssetRef[]) {
  const seen = new Set<string>()
  exp.selectedAssets.value = refs.filter((ref) => {
    const key = selectedRefKey(ref)
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
}

function setSelectedAssets(assets: AssetSummary[]) {
  setSelectedAssetRefs(assets.map(exportRefForAsset))
}

function addSelectedAssets(assets: AssetSummary[]) {
  setSelectedAssetRefs([
    ...exp.selectedAssets.value,
    ...assets.map(exportRefForAsset),
  ])
}

function toggleAssetSelection(asset: AssetSummary) {
  const key = AllAssetsDisplayUtils.getAssetKey(asset)
  if (selectedAssetKeys.value.has(key)) {
    setSelectedAssetRefs(exp.selectedAssets.value.filter(ref => selectedRefKey(ref) !== key))
  } else {
    addSelectedAssets([asset])
  }
  lastSelectionAnchorKey.value = key
}

/** Display helpers for AssetMap rows. */
class AllAssetsDisplayUtils {
  /** Build a stable key because path_id can repeat across different bundles. */
  static getAssetKey(asset: AssetSummary): string {
    return `${asset.source_bundle_path || ''}::${asset.path_id}`
  }

  static getAssetDisplayName(asset: AssetSummary): string {
    return AssetDisplayUtils.getAssetDisplayName(asset)
  }

  static getAssetFullDisplayPath(asset: AssetSummary): string {
    return asset.source_bundle_path || ''
  }

  static getAssetHierarchyPath(asset: AssetSummary): string {
    return asset.path || ''
  }

  /** Active state must include the source bundle path, not only object path_id. */
  static isAssetActive(asset: AssetSummary, selectedAsset: AssetSummary | null): boolean {
    if (!selectedAsset) return false
    return selectedAsset.path_id === asset.path_id &&
      selectedAsset.source_bundle_path === asset.source_bundle_path
  }
}

/** Type filter helpers for the All Assets index view. */
class AllAssetsTypeFilterUtils {
  /** Store key is independent from the file-list filter key. */
  static readonly storeKey = 'all-assets-active-filters'

  /** Keep restored values as clean strings so invalid store data cannot affect the UI. */
  static normalizeStoredFilters(value: unknown): string[] {
    if (!Array.isArray(value)) return []
    return value.filter((item): item is string => typeof item === 'string' && item.length > 0)
  }

  /** Persist the local filter selection through the shared Tauri settings store. */
  static async saveFilters(filters: string[]): Promise<void> {
    const store = await WorkspaceStoreUtils.getStore()
    await store.set(AllAssetsTypeFilterUtils.storeKey, filters)
    await store.save()
  }

  /** Restore the local filter selection after the component is mounted. */
  static async loadFilters(): Promise<string[]> {
    const store = await WorkspaceStoreUtils.getStore()
    return AllAssetsTypeFilterUtils.normalizeStoredFilters(
      await store.get(AllAssetsTypeFilterUtils.storeKey),
    )
  }
}

/** Complete persisted advanced filter for the All Assets index view. */
class AllAssetsAdvancedFilterUtils {
  static readonly storeKey = 'all-assets-advanced-filter'

  static async saveFilter(filter: AdvancedAssetFilterState): Promise<void> {
    const store = await WorkspaceStoreUtils.getStore()
    await store.set(AllAssetsAdvancedFilterUtils.storeKey, normalizeAdvancedAssetFilter(filter))
    await store.save()
  }

  static async loadFilter(): Promise<AdvancedAssetFilterState | null> {
    const store = await WorkspaceStoreUtils.getStore()
    const saved = await store.get(AllAssetsAdvancedFilterUtils.storeKey)
    if (!saved || typeof saved !== 'object') return null
    return normalizeAdvancedAssetFilter(saved)
  }
}

/** Persisted search text for the All Assets index view. */
class AllAssetsSearchQueryUtils {
  static readonly storeKey = 'all-assets-search-query'

  static normalizeStoredQuery(value: unknown): string {
    return typeof value === 'string' ? value : ''
  }

  static async saveQuery(value: string): Promise<void> {
    const store = await WorkspaceStoreUtils.getStore()
    await store.set(AllAssetsSearchQueryUtils.storeKey, value)
    await store.save()
  }

  static async loadQuery(): Promise<string> {
    const store = await WorkspaceStoreUtils.getStore()
    return AllAssetsSearchQueryUtils.normalizeStoredQuery(
      await store.get(AllAssetsSearchQueryUtils.storeKey),
    )
  }
}

/** AssetMap index command helpers for the All Assets tab. */
class AllAssetsMapQueryUtils {
  /** Read summary first; missing summary means Build Index has not created an index. */
  static async loadSummary(workspaceDirectory: string): Promise<MapSummary | null> {
    const assetMapCacheRoot = await AssetMapCacheUtils.getCacheRoot()
    return await invoke<MapSummary | null>('get_map_summary', {
      workspaceDir: workspaceDirectory,
      assetMapCacheRoot,
    })
  }

  /** Read type counts for the filter popup from the AssetMap index. */
  static async loadClassStats(workspaceDirectory: string): Promise<MapAssetClassStat[]> {
    const assetMapCacheRoot = await AssetMapCacheUtils.getCacheRoot()
    return await invoke<MapAssetClassStat[]>('get_map_asset_class_stats', {
      workspaceDir: workspaceDirectory,
      assetMapCacheRoot,
    })
  }

  /** Read optional FTS search-index status for fast contains filters. */
  static async loadSearchIndexStatus(workspaceDirectory: string, silent = false): Promise<AssetSearchIndexStatus> {
    const assetMapCacheRoot = await AssetMapCacheUtils.getCacheRoot()
    return await invoke<AssetSearchIndexStatus>('get_asset_search_index_status', {
      workspaceDir: workspaceDirectory,
      assetMapCacheRoot,
      silent,
    })
  }

  /** Start background search-index backfill; detailed progress is sent to the log panel. */
  static async startSearchIndexBuild(workspaceDirectory: string): Promise<void> {
    const assetMapCacheRoot = await AssetMapCacheUtils.getCacheRoot()
    await invoke('build_asset_search_index', {
      workspaceDir: workspaceDirectory,
      assetMapCacheRoot,
    })
  }

  /** Query all matching AssetMap rows using current search text and type filters. */
  static async queryAssets(options: {
    workspaceDirectory: string
    search: string
    classNames: string[]
  }): Promise<MapAssetQueryResult> {
    const assetMapCacheRoot = await AssetMapCacheUtils.getCacheRoot()
    return await invoke<MapAssetQueryResult>('query_map_assets', {
      workspaceDir: options.workspaceDirectory,
      assetMapCacheRoot,
      options: {
        search: options.search,
        class_names: options.classNames,
        ...buildAdvancedFilterQueryOptions(),
        offset: 0,
        limit: ASSET_MAP_FULL_QUERY_LIMIT,
        sort_by: advancedFilter.value.sortBy,
        sort_direction: advancedFilter.value.sortDirection,
      },
    })
  }
}

/** Cache helpers keep the tab warm across left-tab switches. */
class AllAssetsCacheUtils {
  /** Persist current reactive state into module-level cache. */
  static save(options: {
    workspaceDirectory: string
    query: string
    activeTypeFilters: string[]
    advancedFilter: AdvancedAssetFilterState
    mapSummary: MapSummary | null
    searchIndexStatus: AssetSearchIndexStatus | null
    typeStats: MapAssetClassStat[]
    assets: AssetSummary[]
    totalAssets: number
    loadError: string
    visibleAssetCount: number
    collapsedGroups: string[]
  }): void {
    AllAssetsPageCache.workspaceDirectory = options.workspaceDirectory
    AllAssetsPageCache.query = options.query
    AllAssetsPageCache.activeTypeFilters = [...options.activeTypeFilters]
    AllAssetsPageCache.advancedFilter = {
      ...options.advancedFilter,
      classNames: [...options.advancedFilter.classNames],
    }
    AllAssetsPageCache.mapSummary = options.mapSummary
    AllAssetsPageCache.searchIndexStatus = options.searchIndexStatus
      ? { ...options.searchIndexStatus }
      : null
    AllAssetsPageCache.typeStats = [...options.typeStats]
    AllAssetsPageCache.assets = [...options.assets]
    AllAssetsPageCache.totalAssets = options.totalAssets
    AllAssetsPageCache.visibleAssetCount = options.visibleAssetCount
    AllAssetsPageCache.collapsedGroups = [...options.collapsedGroups]
    AllAssetsPageCache.loadError = options.loadError
    AllAssetsPageCache.initialized = true
  }

  /** Check whether cached data belongs to the current workspace. */
  static matchesWorkspace(workspaceDirectory: string): boolean {
    return AllAssetsPageCache.initialized &&
      AllAssetsPageCache.workspaceDirectory === workspaceDirectory
  }

  /** Any initialized cache can be shown immediately during tab remount. */
  static hasInitializedCache(): boolean {
    return AllAssetsPageCache.initialized
  }

  /** Restore cached values into the current component instance. */
  static restore(): void {
    query.value = AllAssetsPageCache.query
    activeTypeFilters.value = [...AllAssetsPageCache.activeTypeFilters]
    if (AllAssetsPageCache.advancedFilter) {
      advancedFilter.value = normalizeAdvancedAssetFilter(AllAssetsPageCache.advancedFilter)
    }
    mapSummary.value = AllAssetsPageCache.mapSummary
    searchIndexStatus.value = AllAssetsPageCache.searchIndexStatus
      ? { ...AllAssetsPageCache.searchIndexStatus }
      : null
    typeStats.value = [...AllAssetsPageCache.typeStats]
    assets.value = [...AllAssetsPageCache.assets]
    totalAssets.value = AllAssetsPageCache.totalAssets
    visibleAssetCount.value = AllAssetsPageCache.visibleAssetCount
    collapsedGroups.value = new Set(AllAssetsPageCache.collapsedGroups)
    loadError.value = AllAssetsPageCache.loadError
    loading.value = false
    querying.value = false
  }

  /** Drop stale cache when AssetMap is rebuilt, cleared, or workspace changed. */
  static clear(): void {
    AllAssetsPageCache.workspaceDirectory = ''
    AllAssetsPageCache.mapSummary = null
    AllAssetsPageCache.searchIndexStatus = null
    AllAssetsPageCache.advancedFilter = null
    AllAssetsPageCache.typeStats = []
    AllAssetsPageCache.assets = []
    AllAssetsPageCache.totalAssets = 0
    AllAssetsPageCache.visibleAssetCount = ALL_ASSETS_RENDER_CHUNK_SIZE
    AllAssetsPageCache.collapsedGroups = []
    AllAssetsPageCache.loadError = ''
    AllAssetsPageCache.initialized = false
  }
}

/** Rows already filtered by the AssetMap query. */
const displayedAssets = computed(() => assets.value)

function resetVisibleAssetCount() {
  visibleAssetCount.value = ALL_ASSETS_RENDER_CHUNK_SIZE
}

function showMoreVisibleAssets() {
  if (visibleAssetCount.value >= navigableDisplayedAssets.value.length) return
  visibleAssetCount.value = Math.min(
    visibleAssetCount.value + ALL_ASSETS_RENDER_CHUNK_SIZE,
    navigableDisplayedAssets.value.length,
  )
  savePageCache()
}

function handleAllAssetsScroll(event: Event) {
  const element = event.target as HTMLElement | null
  if (!element) return
  const { scrollTop, scrollHeight, clientHeight } = element
  if (scrollHeight - scrollTop - clientHeight < 180) {
    showMoreVisibleAssets()
  }
}

function activateAllAssetsItem(asset: AssetSummary) {
  focusAllAssetsList()
  ws.activateAsset(asset)
}

function selectAssetRange(targetAsset: AssetSummary) {
  const list = navigableDisplayedAssets.value
  const targetKey = AllAssetsDisplayUtils.getAssetKey(targetAsset)
  const anchorKey = lastSelectionAnchorKey.value || targetKey
  const anchorIndex = list.findIndex(asset => AllAssetsDisplayUtils.getAssetKey(asset) === anchorKey)
  const targetIndex = list.findIndex(asset => AllAssetsDisplayUtils.getAssetKey(asset) === targetKey)
  if (targetIndex < 0) return
  const start = anchorIndex >= 0 ? Math.min(anchorIndex, targetIndex) : targetIndex
  const end = anchorIndex >= 0 ? Math.max(anchorIndex, targetIndex) : targetIndex
  addSelectedAssets(list.slice(start, end + 1))
  lastSelectionAnchorKey.value = targetKey
}

function handleAssetItemClick(event: MouseEvent, asset: AssetSummary) {
  if (suppressNextAssetClick) {
    suppressNextAssetClick = false
    event.preventDefault()
    return
  }
  if (event.shiftKey) {
    event.preventDefault()
    selectAssetRange(asset)
    return
  }
  if (event.ctrlKey || event.metaKey) {
    event.preventDefault()
    toggleAssetSelection(asset)
    return
  }
  activateAllAssetsItem(asset)
}

function handleCheckboxClick(event: MouseEvent, asset: AssetSummary) {
  event.stopPropagation()
  if (event.shiftKey) {
    selectAssetRange(asset)
    return
  }
  toggleAssetSelection(asset)
}

function shouldIgnoreSelectionDrag(event: PointerEvent): boolean {
  const target = event.target as HTMLElement | null
  if (!target) return true
  return Boolean(target.closest([
    '.asset-group-title',
    'button',
    'input',
    'textarea',
    'select',
    '.el-input',
    '.el-button',
    '.el-popper',
  ].join(',')))
}

function handleAllAssetsPointerDown(event: PointerEvent) {
  focusAllAssetsList()
  if (event.button !== 0 || shouldIgnoreSelectionDrag(event)) return
  selectionDrag.value = {
    originX: event.clientX,
    originY: event.clientY,
    currentX: event.clientX,
    currentY: event.clientY,
    active: false,
    additive: event.ctrlKey || event.metaKey,
  }
  document.addEventListener('pointermove', handleSelectionPointerMove)
  document.addEventListener('pointerup', handleSelectionPointerUp, { once: true })
}

function handleSelectionPointerMove(event: PointerEvent) {
  const drag = selectionDrag.value
  if (!drag) return
  drag.currentX = event.clientX
  drag.currentY = event.clientY
  if (!drag.active && Math.hypot(drag.currentX - drag.originX, drag.currentY - drag.originY) > 4) {
    drag.active = true
    suppressNextAssetClick = true
  }
}

function handleSelectionPointerUp() {
  const drag = selectionDrag.value
  document.removeEventListener('pointermove', handleSelectionPointerMove)
  if (!drag) return
  selectionDrag.value = null
  if (!drag.active) return

  const selectionRect = normalizedDragRect(drag)
  suppressNextAssetClick = false
  const selectedKeys = new Set<string>()
  ws.allAssetsListRef.value
    ?.querySelectorAll<HTMLElement>('[data-asset-key]')
    .forEach((element) => {
      const rect = element.getBoundingClientRect()
      if (rectIntersects(selectionRect, rect)) {
        const key = element.dataset.assetKey
        if (key) selectedKeys.add(key)
      }
    })
  if (selectedKeys.size === 0) return

  const selectedByDrag = navigableDisplayedAssets.value.filter(asset => selectedKeys.has(AllAssetsDisplayUtils.getAssetKey(asset)))
  if (drag.additive) {
    addSelectedAssets(selectedByDrag)
  } else {
    setSelectedAssets(selectedByDrag)
  }
  lastSelectionAnchorKey.value = AllAssetsDisplayUtils.getAssetKey(selectedByDrag[selectedByDrag.length - 1])
}

function normalizedDragRect(drag: NonNullable<typeof selectionDrag.value>) {
  const left = Math.min(drag.originX, drag.currentX)
  const top = Math.min(drag.originY, drag.currentY)
  const right = Math.max(drag.originX, drag.currentX)
  const bottom = Math.max(drag.originY, drag.currentY)
  return { left, top, right, bottom, width: right - left, height: bottom - top }
}

function rectIntersects(
  selection: { left: number; right: number; top: number; bottom: number },
  target: DOMRect,
): boolean {
  return selection.left <= target.right &&
    selection.right >= target.left &&
    selection.top <= target.bottom &&
    selection.bottom >= target.top
}

const selectionBoxStyle = computed(() => {
  const drag = selectionDrag.value
  if (!drag?.active) return {}
  const rect = normalizedDragRect(drag)
  return {
    left: `${rect.left}px`,
    top: `${rect.top}px`,
    width: `${rect.width}px`,
    height: `${rect.height}px`,
  }
})

/** Group loaded index rows by class name, similar to the normal asset list. */
const allGroupedAssets = computed(() => {
  const groups: Record<string, AssetSummary[]> = {}
  for (const asset of displayedAssets.value) {
    if (!groups[asset.class_name]) groups[asset.class_name] = []
    groups[asset.class_name].push(asset)
  }
  return groups
})

/** Sort result groups by loaded result count, then by class name for stable display. */
const classNames = computed(() => {
  return Object.entries(allGroupedAssets.value)
    .sort((left, right) => {
      if (right[1].length !== left[1].length) return right[1].length - left[1].length
      return left[0].localeCompare(right[0])
    })
    .map(([className]) => className)
})

const useFlatAssetList = computed(() => advancedFilter.value.sortBy !== 'type')

const navigableDisplayedAssets = computed(() => {
  if (useFlatAssetList.value) return displayedAssets.value
  return classNames.value
    .filter(className => !collapsedGroups.value.has(className))
    .flatMap(className => allGroupedAssets.value[className] || [])
})

const hasMoreAssets = computed(() => visibleAssetCount.value < navigableDisplayedAssets.value.length)

const visibleFlatAssets = computed(() => displayedAssets.value.slice(0, visibleAssetCount.value))

const visibleGroupedAssets = computed(() => {
  const groups: Record<string, AssetSummary[]> = {}
  let remaining = visibleAssetCount.value
  for (const className of classNames.value) {
    const group = allGroupedAssets.value[className] || []
    if (collapsedGroups.value.has(className)) {
      groups[className] = []
      continue
    }
    const visibleGroup = remaining > 0 ? group.slice(0, remaining) : []
    groups[className] = visibleGroup
    remaining -= visibleGroup.length
  }
  return groups
})

function selectedDisplayedAssetIndex(): number {
  const selected = ws.selectedAsset.value
  if (!selected) return -1
  const selectedKey = AllAssetsDisplayUtils.getAssetKey(selected)
  return navigableDisplayedAssets.value.findIndex(asset => AllAssetsDisplayUtils.getAssetKey(asset) === selectedKey)
}

async function ensureDisplayedAssetLoaded(index: number) {
  while (index >= visibleNavigableDisplayedAssets().length && hasMoreAssets.value) {
    const before = visibleAssetCount.value
    showMoreVisibleAssets()
    await nextTick()
    if (visibleAssetCount.value <= before) break
  }
}

function visibleNavigableDisplayedAssets(): AssetSummary[] {
  const visibleKeys = new Set([
    ...visibleFlatAssets.value,
    ...Object.values(visibleGroupedAssets.value).flat(),
  ].map(asset => AllAssetsDisplayUtils.getAssetKey(asset)))
  return navigableDisplayedAssets.value.filter(asset => visibleKeys.has(AllAssetsDisplayUtils.getAssetKey(asset)))
}

async function selectDisplayedAssetByIndex(index: number) {
  if (index < 0) return
  focusAllAssetsList()
  await ensureDisplayedAssetLoaded(index)
  if (index >= navigableDisplayedAssets.value.length) return
  const asset = navigableDisplayedAssets.value[index]
  if (!asset) return
  await ws.activateAsset(asset)
  await nextTick()
  const key = AllAssetsDisplayUtils.getAssetKey(asset)
  const target = ws.allAssetsListRef.value?.querySelector(`[data-asset-key="${CSS.escape(key)}"]`) as HTMLElement | null
  target?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
}

function handleAllAssetsKeydown(event: KeyboardEvent): boolean {
  if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return false
  const list = navigableDisplayedAssets.value
  if (list.length === 0) return false
  event.preventDefault()
  const currentIndex = selectedDisplayedAssetIndex()
  const nextIndex = event.key === 'ArrowDown'
    ? (currentIndex < 0
        ? 0
        : currentIndex < list.length - 1
          ? currentIndex + 1
          : visibleAssetCount.value < list.length
            ? visibleAssetCount.value
            : 0)
    : (currentIndex > 0 ? currentIndex - 1 : list.length - 1)
  void selectDisplayedAssetByIndex(nextIndex)
  return true
}

function isAllAssetsGroupCollapsed(className: string): boolean {
  return collapsedGroups.value.has(className)
}

function toggleAllAssetsGroup(className: string) {
  const next = new Set(collapsedGroups.value)
  if (next.has(className)) {
    next.delete(className)
  } else {
    next.add(className)
  }
  collapsedGroups.value = next
  savePageCache()
}

/** Counter text remains compact enough for the panel header. */
const assetCounterText = computed(() => {
  if (!mapSummary.value) return t('allAssets.mapRequired')
  return t('allAssets.assetCounter', {
    loaded: Math.min(visibleAssetCount.value, navigableDisplayedAssets.value.length),
    total: displayedAssets.value.length,
  })
})

const searchIndexHintText = computed(() => {
  const status = searchIndexStatus.value
  if (!mapSummary.value || !status || status.ready) return ''
  if (searchIndexBuildRequested.value) {
    return t('allAssets.searchIndexBuilding', {
      indexed: status.indexed_count,
      total: status.asset_count,
    })
  }
  return t('allAssets.searchIndexPending', {
    indexed: status.indexed_count,
    total: status.asset_count,
  })
})

const advancedFilterActiveCount = computed(() => {
  let count = advancedFilter.value.classNames.length
  count += normalizedTextList([
    advancedFilter.value.nameQuery,
    ...advancedFilter.value.nameIncludeQueries,
  ]).length
  count += normalizedTextList([
    advancedFilter.value.nameExcludeQuery,
    ...advancedFilter.value.nameExcludeQueries,
  ]).length
  if (advancedFilter.value.bundleQuery.trim()) count += 1
  if (advancedFilter.value.pathIdQuery.trim()) count += 1
  if (advancedFilter.value.minSizeText.trim()) count += 1
  if (advancedFilter.value.maxSizeText.trim()) count += 1
  if (advancedFilter.value.sortBy !== 'type' || advancedFilter.value.sortDirection !== 'asc') count += 1
  return count
})

function stopSearchIndexStatusPolling() {
  searchIndexPollToken += 1
  if (searchIndexPollTimer !== null) {
    window.clearTimeout(searchIndexPollTimer)
    searchIndexPollTimer = null
  }
}

function scheduleSearchIndexStatusPoll(
  workspaceDirectory: string,
  delayMs = SEARCH_INDEX_POLL_INTERVAL_MS,
  token = searchIndexPollToken,
) {
  if (!workspaceDirectory || searchIndexStatus.value?.ready) return
  if (searchIndexPollTimer !== null) {
    window.clearTimeout(searchIndexPollTimer)
  }
  searchIndexPollTimer = window.setTimeout(() => {
    searchIndexPollTimer = null
    void pollSearchIndexStatus(workspaceDirectory, token)
  }, delayMs)
}

async function pollSearchIndexStatus(workspaceDirectory: string, token: number) {
  if (token !== searchIndexPollToken || workspaceDirectory !== ws.workDir.value || !mapSummary.value) return

  try {
    const status = await AllAssetsMapQueryUtils.loadSearchIndexStatus(workspaceDirectory, true)
    if (token !== searchIndexPollToken || workspaceDirectory !== ws.workDir.value) return
    searchIndexStatus.value = status
    if (status.ready) {
      searchIndexBuildRequested.value = false
      savePageCache()
      return
    }
    savePageCache()
    scheduleSearchIndexStatusPoll(workspaceDirectory, SEARCH_INDEX_POLL_INTERVAL_MS, token)
  } catch (error) {
    if (token !== searchIndexPollToken || workspaceDirectory !== ws.workDir.value) return
    console.error('Failed to poll All Assets search index status:', error)
    scheduleSearchIndexStatusPoll(workspaceDirectory, SEARCH_INDEX_POLL_ERROR_DELAY_MS, token)
  }
}

async function ensureSearchIndexBuildStarted(workspaceDirectory: string) {
  if (!mapSummary.value) return
  try {
    const status = await AllAssetsMapQueryUtils.loadSearchIndexStatus(workspaceDirectory)
    if (workspaceDirectory !== ws.workDir.value) return
    searchIndexStatus.value = status
    if (status.ready) {
      searchIndexBuildRequested.value = false
      savePageCache()
      stopSearchIndexStatusPolling()
    } else {
      searchIndexPollToken += 1
      const token = searchIndexPollToken
      scheduleSearchIndexStatusPoll(workspaceDirectory, SEARCH_INDEX_POLL_INITIAL_DELAY_MS, token)
    }
    if (!status.ready && !searchIndexBuildRequested.value) {
      searchIndexBuildRequested.value = true
      await AllAssetsMapQueryUtils.startSearchIndexBuild(workspaceDirectory)
      if (workspaceDirectory !== ws.workDir.value) return
    }
    savePageCache()
  } catch (error) {
    if (workspaceDirectory !== ws.workDir.value) return
    stopSearchIndexStatusPolling()
    searchIndexBuildRequested.value = false
    savePageCache()
    console.error('Failed to start All Assets search index build:', error)
  }
}

function parseSizeBytes(value: string): number | null {
  const trimmed = value.trim()
  if (!trimmed) return null
  const parsed = Number(trimmed)
  if (!Number.isFinite(parsed) || parsed < 0) return null
  const multiplier = advancedFilter.value.sizeUnit === 'MB'
    ? 1024 * 1024
    : advancedFilter.value.sizeUnit === 'KB'
      ? 1024
      : 1
  return Math.floor(parsed * multiplier)
}

function normalizedTextList(values: string[] | undefined): string[] {
  const seen = new Set<string>()
  const result: string[] = []
  for (const value of values || []) {
    const trimmed = value.trim()
    if (!trimmed) continue
    const key = trimmed.toLocaleLowerCase()
    if (seen.has(key)) continue
    seen.add(key)
    result.push(trimmed)
  }
  return result
}

function buildAdvancedFilterQueryOptions() {
  const includeQueries = normalizedTextList([
    advancedFilter.value.nameQuery,
    ...advancedFilter.value.nameIncludeQueries,
  ])
  const excludeQueries = normalizedTextList([
    advancedFilter.value.nameExcludeQuery,
    ...advancedFilter.value.nameExcludeQueries,
  ])
  return {
    name_query: advancedFilter.value.nameQuery.trim() || undefined,
    name_match_mode: advancedFilter.value.nameMatchMode,
    name_include_queries: includeQueries.length ? includeQueries : undefined,
    name_exclude_query: advancedFilter.value.nameExcludeQuery.trim() || undefined,
    name_exclude_queries: excludeQueries.length ? excludeQueries : undefined,
    bundle_query: advancedFilter.value.bundleQuery.trim() || undefined,
    path_id_query: advancedFilter.value.pathIdQuery.trim() || undefined,
    min_size: parseSizeBytes(advancedFilter.value.minSizeText),
    max_size: parseSizeBytes(advancedFilter.value.maxSizeText),
  }
}

function isTaskCancelledError(error: unknown): boolean {
  return /cancelled|canceled|interrupted|interrupted query|stop requested|query aborted/i.test(String(error))
}

function invalidateActiveQueries() {
  querySequence += 1
  activeQueryToken += 1
  querying.value = false
}

function scheduleLoadAssets(delayMs = 250) {
  invalidateActiveQueries()
  if (mapSummary.value) {
    querying.value = true
  }
  if (searchQueryTimer !== null) {
    window.clearTimeout(searchQueryTimer)
  }
  searchQueryTimer = window.setTimeout(() => {
    searchQueryTimer = null
    void loadAssets()
  }, delayMs)
}

function applyAdvancedFilter(nextFilter?: AdvancedAssetFilterState) {
  if (nextFilter) {
    advancedFilter.value = normalizeAdvancedAssetFilter(nextFilter)
  }
  activeTypeFilters.value = [...advancedFilter.value.classNames]
  resetVisibleAssetCount()
  AllAssetsTypeFilterUtils.saveFilters(activeTypeFilters.value).catch((error) => {
    console.error('Failed to save All Assets filters:', error)
  })
  AllAssetsAdvancedFilterUtils.saveFilter(advancedFilter.value).catch((error) => {
    console.error('Failed to save All Assets advanced filter:', error)
  })
  void loadAssets()
}

function resetAdvancedFilter() {
  advancedFilter.value = createDefaultAdvancedAssetFilter()
  activeTypeFilters.value = []
  collapsedGroups.value = new Set()
  resetVisibleAssetCount()
  AllAssetsTypeFilterUtils.saveFilters([]).catch((error) => {
    console.error('Failed to save All Assets filters:', error)
  })
  AllAssetsAdvancedFilterUtils.saveFilter(advancedFilter.value).catch((error) => {
    console.error('Failed to save All Assets advanced filter:', error)
  })
  void loadAssets()
}

/** Save current panel state into the cross-mount cache. */
function savePageCache() {
  AllAssetsCacheUtils.save({
    workspaceDirectory: ws.workDir.value,
    query: query.value,
    activeTypeFilters: activeTypeFilters.value,
    advancedFilter: advancedFilter.value,
    mapSummary: mapSummary.value,
    searchIndexStatus: searchIndexStatus.value,
    typeStats: typeStats.value,
    assets: assets.value,
    totalAssets: totalAssets.value,
    visibleAssetCount: visibleAssetCount.value,
    collapsedGroups: [...collapsedGroups.value],
    loadError: loadError.value,
  })
}

/** Load summary, filter stats, and the first search page. */
async function refreshFromMap(forceRefresh = false) {
  const workspaceDirectory = ws.workDir.value
  loadError.value = ''

  if (!workspaceDirectory) {
    stopSearchIndexStatusPolling()
    mapSummary.value = null
    searchIndexStatus.value = null
    searchIndexBuildRequested.value = false
    typeStats.value = []
    assets.value = []
    totalAssets.value = 0
    resetVisibleAssetCount()
    collapsedGroups.value = new Set()
    savePageCache()
    return
  }

  if (!forceRefresh && AllAssetsCacheUtils.matchesWorkspace(workspaceDirectory)) {
    if (!searchIndexStatus.value?.ready && !searchIndexBuildRequested.value) {
      void ensureSearchIndexBuildStarted(workspaceDirectory)
    }
    return
  }

  invalidateActiveQueries()
  assets.value = []
  totalAssets.value = 0
  resetVisibleAssetCount()
  loading.value = true
  try {
    mapSummary.value = await AllAssetsMapQueryUtils.loadSummary(workspaceDirectory)
    if (!mapSummary.value) {
      stopSearchIndexStatusPolling()
      searchIndexStatus.value = null
      searchIndexBuildRequested.value = false
      typeStats.value = []
      savePageCache()
      return
    }

    typeStats.value = await AllAssetsMapQueryUtils.loadClassStats(workspaceDirectory)
    await loadAssets()
    void ensureSearchIndexBuildStarted(workspaceDirectory)
  } catch (error) {
    loadError.value = String(error)
  } finally {
    loading.value = false
    savePageCache()
  }
}

/** Load every matching row into memory; rendering remains chunked by scroll position. */
async function loadAssets() {
  const workspaceDirectory = ws.workDir.value
  if (!workspaceDirectory || !mapSummary.value) return

  const queryToken = ++activeQueryToken
  const currentQuerySequence = ++querySequence
  querying.value = true

  try {
    const result = await AllAssetsMapQueryUtils.queryAssets({
      workspaceDirectory,
      search: query.value.trim(),
      classNames: activeTypeFilters.value,
    })
    if (queryToken !== activeQueryToken || currentQuerySequence !== querySequence) return
    totalAssets.value = result.total
    assets.value = result.assets
    resetVisibleAssetCount()
    loadError.value = ''
    savePageCache()
  } catch (error) {
    if (queryToken !== activeQueryToken || currentQuerySequence !== querySequence) return
    if (isTaskCancelledError(error)) {
      loadError.value = ''
    } else {
      loadError.value = String(error)
    }
    savePageCache()
  } finally {
    if (queryToken === activeQueryToken && currentQuerySequence === querySequence) {
      querying.value = false
    }
  }
}

/** Restore remembered filters and load the initial index page. */
onMounted(async () => {
  unregisterAllAssetsKeyboardNavigator = ws.registerKeyboardNavigator('all-assets', handleAllAssetsKeydown)

  if (AllAssetsCacheUtils.matchesWorkspace(ws.workDir.value)) {
    if (!searchIndexStatus.value?.ready) {
      void ensureSearchIndexBuildStarted(ws.workDir.value)
    }
    return
  }

  if (AllAssetsCacheUtils.hasInitializedCache()) {
    AllAssetsCacheUtils.clear()
  }

  const storedAdvancedFilter = await AllAssetsAdvancedFilterUtils.loadFilter()
  if (storedAdvancedFilter) {
    advancedFilter.value = storedAdvancedFilter
    activeTypeFilters.value = [...storedAdvancedFilter.classNames]
  } else {
    activeTypeFilters.value = await AllAssetsTypeFilterUtils.loadFilters()
    advancedFilter.value = {
      ...createDefaultAdvancedAssetFilter(),
      classNames: [...activeTypeFilters.value],
    }
  }
  query.value = await AllAssetsSearchQueryUtils.loadQuery()
  await refreshFromMap(true)
})

/** Reload the index view if the user switches workspace directories. */
watch(() => ws.workDir.value, () => {
  stopSearchIndexStatusPolling()
  AllAssetsCacheUtils.clear()
  searchIndexBuildRequested.value = false
  void refreshFromMap(true)
})

/** Reload the index view when Build Index or Clear Index changes the AssetMap index. */
watch(() => ws.assetMapVersion.value, () => {
  stopSearchIndexStatusPolling()
  AllAssetsCacheUtils.clear()
  searchIndexBuildRequested.value = false
  void refreshFromMap(true)
})

watch(query, () => {
  resetVisibleAssetCount()
  savePageCache()
  AllAssetsSearchQueryUtils.saveQuery(query.value).catch((error) => {
    console.error('Failed to save All Assets search query:', error)
  })
  scheduleLoadAssets()
})

watch(() => logSystem.stopSignal.value, (nextSignal) => {
  if (nextSignal === lastStopSignal) return
  lastStopSignal = nextSignal
  invalidateActiveQueries()
  stopSearchIndexStatusPolling()
  searchIndexBuildRequested.value = false
})

onUnmounted(() => {
  stopSearchIndexStatusPolling()
  if (searchQueryTimer !== null) {
    window.clearTimeout(searchQueryTimer)
    searchQueryTimer = null
  }
  document.removeEventListener('pointermove', handleSelectionPointerMove)
  unregisterAllAssetsKeyboardNavigator?.()
  unregisterAllAssetsKeyboardNavigator = null
})

</script>

<template>
  <div class="all-assets-panel">
    <!-- Panel Header -->
    <div class="pane-header">
      <span class="pane-header-left">{{ t('allAssets.title') }}</span>
      <span class="pane-header-right">
        <AdvancedAssetFilterCard
          v-model:visible="advancedFilterVisible"
          v-model="advancedFilter"
          :type-stats="typeStats"
          :disabled="!mapSummary || loading || querying"
          :active-count="advancedFilterActiveCount"
          @apply="applyAdvancedFilter"
          @reset="resetAdvancedFilter"
        />
      </span>
      <span class="panel-count">{{ assetCounterText }}</span>
    </div>

    <!-- Search Input -->
    <div class="asset-search-bar">
      <el-input
        v-model="query"
        :placeholder="t('allAssets.searchPlaceholder')"
        size="small"
        clearable
        :disabled="!mapSummary"
        :prefix-icon="Search"
        class="asset-search-input"
      />
    </div>

    <div v-if="searchIndexHintText" class="all-assets-search-index-hint">
      <el-icon class="is-loading" :size="14"><Refresh /></el-icon>
      <span>{{ searchIndexHintText }}</span>
    </div>

    <div v-if="exp.selectedAssets.value.length > 0" class="selection-bar">
      <span class="selection-text">
        {{ t('home.selectedCount', { count: exp.selectedAssets.value.length }) }}
      </span>
      <span class="selection-hint">{{ t('home.multiSelectHint') }}</span>
      <el-button size="small" text type="danger" @click="clearSelection()">{{ t('home.clearSelection') }}</el-button>
    </div>

    <!-- Build Index required hint -->
    <div v-if="!ws.workDir.value" class="no-data-hint">
      <el-icon :size="32"><FolderOpened /></el-icon>
      <p>{{ t('allAssets.selectWorkspaceFirst') }}</p>
      <p class="no-data-sub">{{ t('allAssets.databaseHint') }}</p>
    </div>

    <div v-else-if="loading && assets.length === 0" class="no-data-hint">
      <el-icon class="is-loading" :size="32"><Refresh /></el-icon>
      <p>{{ t('allAssets.loadingAssets') }}</p>
    </div>

    <div v-else-if="!mapSummary" class="no-data-hint">
      <el-icon :size="32"><FolderOpened /></el-icon>
      <p>{{ t('allAssets.buildMapRequired') }}</p>
      <p class="no-data-sub">{{ t('allAssets.buildMapHint') }}</p>
      <p v-if="loadError" class="no-data-error">{{ loadError }}</p>
    </div>

    <!-- Asset List -->
    <div
      v-else
      :ref="setAllAssetsListElement"
      class="pane-scroll pane-assets"
      tabindex="0"
      @focusin="ws.setKeyboardNavigationScope('all-assets')"
      @pointerdown="handleAllAssetsPointerDown"
      @keydown="handleAllAssetsKeydown"
      @scroll="handleAllAssetsScroll"
    >
      <div v-if="querying" class="all-assets-querying-bar">
        <el-icon class="is-loading" :size="14"><Refresh /></el-icon>
        <span>{{ t('allAssets.searching') }}</span>
      </div>

      <!-- No match -->
      <div v-if="displayedAssets.length === 0 && !querying" class="panel-empty">
        {{ t('common.noMatchingAssets') }}
      </div>

      <template v-if="useFlatAssetList">
        <AssetListItem
          v-for="asset in visibleFlatAssets"
          :key="AllAssetsDisplayUtils.getAssetKey(asset)"
          :asset="asset"
          :asset-key="AllAssetsDisplayUtils.getAssetKey(asset)"
          :active="AllAssetsDisplayUtils.isAssetActive(asset, ws.selectedAsset.value)"
          :selected="isAssetSelected(asset)"
          :selectable="true"
          :bundle-path="AllAssetsDisplayUtils.getAssetFullDisplayPath(asset)"
          :hierarchy-path="AllAssetsDisplayUtils.getAssetHierarchyPath(asset)"
          :size-text="ws.getAssetSize(asset)"
          :show-class="true"
          @click="handleAssetItemClick($event, asset)"
          @contextmenu.prevent="ws.showContextMenu($event, asset)"
          @checkbox-click="handleCheckboxClick($event, asset)"
        />
      </template>

      <div
        v-else
        v-for="className in classNames"
        :key="className"
        class="asset-group"
      >
        <div
          class="asset-group-title"
          :class="{ 'asset-group-collapsed': isAllAssetsGroupCollapsed(className) }"
          @click="toggleAllAssetsGroup(className)"
        >
          <span class="asset-group-arrow">{{ isAllAssetsGroupCollapsed(className) ? '+' : '-' }}</span>
          <el-icon :size="16"><component :is="ClassIconUtils.getClassIcon(className)" /></el-icon>
          {{ className }}
          <span class="asset-group-count">
            ({{ allGroupedAssets[className].length }})
          </span>
        </div>

        <!-- AssetMap index result rows -->
        <AssetListItem
          v-for="asset in visibleGroupedAssets[className]"
          :key="AllAssetsDisplayUtils.getAssetKey(asset)"
          :asset="asset"
          :asset-key="AllAssetsDisplayUtils.getAssetKey(asset)"
          :active="AllAssetsDisplayUtils.isAssetActive(asset, ws.selectedAsset.value)"
          :selected="isAssetSelected(asset)"
          :selectable="true"
          :bundle-path="AllAssetsDisplayUtils.getAssetFullDisplayPath(asset)"
          :hierarchy-path="AllAssetsDisplayUtils.getAssetHierarchyPath(asset)"
          :size-text="ws.getAssetSize(asset)"
          @click="handleAssetItemClick($event, asset)"
          @contextmenu.prevent="ws.showContextMenu($event, asset)"
          @checkbox-click="handleCheckboxClick($event, asset)"
        />
      </div>

      <div
        v-if="selectionDrag?.active"
        class="asset-selection-box"
        :style="selectionBoxStyle"
      />
    </div>
  </div>

  <AssetContextMenu :assets="displayedAssets" />

</template>

<style scoped>
.all-assets-panel {
  height: 100%;
  display: flex;
  flex-direction: column;
  position: relative;
  overflow: hidden;
}

.no-data-hint {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  flex: 1;
  gap: 9px;
  color: var(--el-text-color-secondary);
  margin: 14px;
  padding: 44px 22px;
  text-align: center;
  border: 1px dashed color-mix(in srgb, var(--app-border) 86%, transparent);
  border-radius: 14px;
  background:
    radial-gradient(circle at 50% 0%, color-mix(in srgb, var(--el-color-primary) 10%, transparent), transparent 45%),
    var(--app-surface-soft);
}

.no-data-sub {
  font-size: 12px;
  opacity: 0.6;
}

.no-data-error {
  max-width: 90%;
  color: var(--el-color-danger);
  font-size: 12px;
  word-break: break-word;
}

.all-assets-querying-bar {
  position: sticky;
  top: 0;
  z-index: 2;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 28px;
  color: var(--el-color-primary);
  background: color-mix(in srgb, var(--el-color-primary) 12%, var(--app-surface));
  border-bottom: 1px solid color-mix(in srgb, var(--el-color-primary) 26%, transparent);
  font-size: 12px;
}

.all-assets-search-index-hint {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 26px;
  padding: 4px 10px;
  color: var(--el-color-warning);
  background: color-mix(in srgb, var(--el-color-warning) 10%, var(--app-surface));
  border-bottom: 1px solid color-mix(in srgb, var(--el-color-warning) 22%, transparent);
  font-size: 12px;
}

.selection-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  background: color-mix(in srgb, var(--el-color-success) 12%, var(--app-surface));
  border-bottom: 1px solid color-mix(in srgb, var(--el-color-success) 28%, transparent);
  font-size: 11px;
}

.selection-text {
  font-weight: 600;
  color: var(--el-color-success);
}

.selection-hint {
  color: var(--el-text-color-placeholder);
  flex: 1;
}

.asset-selection-box {
  position: fixed;
  z-index: 9999;
  pointer-events: none;
  border: 1px solid var(--el-color-primary);
  background: color-mix(in srgb, var(--el-color-primary) 16%, transparent);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--el-color-primary) 18%, transparent) inset;
}

.asset-search-bar {
  flex-shrink: 0;
  padding: 8px;
  background: var(--app-surface);
  border-bottom: 1px solid var(--app-border-soft);
}

.asset-search-input {
  width: 100%;
}

.pane-header-left {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  min-width: 0;
  overflow: hidden;
}

</style>
