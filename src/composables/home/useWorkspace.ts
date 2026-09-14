/**
 * useWorkspace ->WorkSpace page core business logic composable
 *
 * Contains all state and operations related to Unity AssetBundle browsing:
 *   - Work directory management
 *   - Bundle file list scanning, selection, parsing
 *   - Asset selection, preview (Mesh geometry / texture / text)
 *   - Type filter scanning (Rust-side parallel + MD5 persistent cache)
 *   - File list chunked loading + sorting/filtering
 *   - Keyboard navigation, collapsed groups
 *
 * Uses module-level singleton pattern, all child components share the same state.
 *
 * Usage:
 *   const ws = useWorkspace()
 *   ws.selectWorkDir()
 *   ws.selectBundle(someFile)
 */

import { ref, nextTick, watch } from 'vue'
import { ElMessageBox } from 'element-plus'
import { open } from '@tauri-apps/plugin-dialog'
import { invoke, Channel } from '@tauri-apps/api/core'
import { tempDir } from '@tauri-apps/api/path'
import { i18n } from '../../i18n'
import { useLogSystem } from '../useLogSystem'
import { BundleMetaCacheUtils } from './BundleMetaCacheUtils'
import { WorkspacePreviewUtils, type AssetPreviewOptions } from './WorkspacePreviewUtils'
import { WorkspaceTypeScanUtils } from './WorkspaceTypeScanUtils'
import { WorkspaceComputedUtils } from './WorkspaceComputedUtils'
import { WorkspaceBundleParseUtils } from './WorkspaceBundleParseUtils'
import { WorkspaceDumpUtils } from './WorkspaceDumpUtils'
import { WorkspaceWatchUtils } from './WorkspaceWatchUtils'
import { WorkspaceReturnUtils } from './WorkspaceReturnUtils'
import { WorkspaceStoreUtils } from './WorkspaceStoreUtils'
import { WorkspacePathUtils } from './WorkspacePathUtils'
import { workspaceAssetMapClient } from './WorkspaceAssetMapClient'
import { createWorkspacePreviewState, MESH_PREVIEW_CACHE_LIMIT, TEXTURE_PREVIEW_CACHE_LIMIT } from './WorkspacePreviewState'
import { createWorkspaceAssetMapState, ASSET_MAP_FULL_QUERY_LIMIT } from './WorkspaceAssetMapState'
import type { AdvancedAssetFilterState, BundleFileEntry, BundleMeta, AssetSummary, TypeCacheEntry, MapAssetClassStat } from '../../types'
import { ALL_ASSET_TYPES } from '../../types'
import { AssetMapCacheUtils } from '../../utils/AssetMapCacheUtils'
import { AssetDisplayUtils } from '../../utils/AssetDisplayUtils'
import { createDefaultAdvancedAssetFilter, normalizeAdvancedAssetFilter } from '../../utils/AdvancedAssetFilterUtils'

type SortDirection = 'none' | 'asc' | 'desc'
type KeyboardNavigationScope = 'files' | 'bundle-assets' | 'all-assets'
type KeyboardNavigator = (event: KeyboardEvent) => boolean

interface ClearPreviewCacheResult {
  removed_paths: number
  removed_bytes: number
  details: string[]
}

const WORKSPACE_HISTORY_KEY = 'workspace-dir-history'
const LAST_WORKSPACE_DIR_KEY = 'workspace-dir'
const MAX_WORKSPACE_HISTORY = 30

function nextSortDirection(direction: SortDirection): SortDirection {
  if (direction === 'none') return 'asc'
  if (direction === 'asc') return 'desc'
  return 'none'
}

function arraysEqual(left: string[], right: string[]): boolean {
  if (left.length !== right.length) return false
  return left.every((value, index) => value === right[index])
}

/** Current work directory path */
const workDir = ref('')

/** Persisted workspace directory history shown in the toolbar dropdown. */
const workspaceHistory = ref<string[]>([])

/** .bundle file list */
const bundleFiles = ref<BundleFileEntry[]>([])

/** Currently selected bundle file path */
const selectedBundlePath = ref('')

/** Currently selected bundle metadata */
const bundleMeta = ref<BundleMeta | null>(null)

/** Whether loading is in progress (scanning/parsing) */
const isLoading = ref(false)

/** Loading status text */
const loadingText = ref('')

/** Last model preview scope chosen by the user, reused as dependency-export defaults. */
const lastModelPreviewOptions = ref<AssetPreviewOptions>(WorkspacePreviewUtils.defaultPreviewOptions())

let bundleFileScanRunId = 0


const previewState = createWorkspacePreviewState()
const {
  selectedAsset,
  previewTargetAsset,
  dumpTargetAsset,
  assetProperties,
  previewFilePath,
  meshGeometryData,
  meshCache,
  textureCache,
  textureInfo,
  typedPreviewData,
  typedPreviewCache,
  previewAnimators,
  previewAnimatorCache,
  previewAnimationClips,
  previewAnimationClipCache,
  selectedPreviewAnimationClip,
  selectedPreviewMeshScope,
  textureByteSizeCache,
  previewLoading,
  previewError,
  rightTab,
  dumpData,
  dumpLoading,
  dumpError,
  dumpCache,
  cacheDir,
  contextMenuVisible,
  contextMenuX,
  contextMenuY,
  contextMenuAsset,
  dialogAsset,
  showPropsDialog,
} = previewState

/** Scene hierarchy tree data */
const sceneHierarchy = ref<import('../../types').SceneNode[]>([])
const hierarchyLoading = ref(false)
/** Bundle path for which hierarchy has already been loaded (avoid duplicate loading) */
let hierarchyLoadedBundlePath = ''


// Bundle metadata cache (LRU, memory limit configurable)
// Avoids re-parsing when switching between .bundle files frequently

/** Bundle metadata cache: filePath -> BundleMeta */
const bundleMetaCache = new Map<string, BundleMeta>()

/** Current cache bytes used (rough estimate) */
let cacheUsedBytes = 0

/** Cache max bytes (default 8GB) */
let cacheMaxBytes = 8 * 1024 * 1024 * 1024

/** Cache helpers keep LRU details in BundleMetaCacheUtils while preserving old method names. */
const getCachedBundleMeta = (path: string): BundleMeta | undefined =>
  BundleMetaCacheUtils.getCachedBundleMeta(bundleMetaCache, path)
const getAllCachedAssets = (): AssetSummary[] => BundleMetaCacheUtils.getAllCachedAssets(bundleMetaCache)
const estimateBundleMetaSize = (meta: BundleMeta): number => BundleMetaCacheUtils.estimateBundleMetaSize(meta)
function setCachedBundleMeta(path: string, meta: BundleMeta) {
  cacheUsedBytes = BundleMetaCacheUtils.setCachedBundleMeta(
    bundleMetaCache,
    path,
    meta,
    cacheUsedBytes,
    cacheMaxBytes,
  )
}

const getBundleCacheStats = () => BundleMetaCacheUtils.getBundleCacheStats(bundleMetaCache, cacheUsedBytes, cacheMaxBytes)

/**
 * Load cache max bytes config from store.
 */
async function loadBundleCacheConfig() {
  const s = await WorkspaceStoreUtils.getStore()
  const saved = await s.get('bundle-cache-max-bytes')
  if (typeof saved === 'number' && saved > 0) {
    cacheMaxBytes = Math.min(saved, 48 * 1024 * 1024 * 1024) // Max 48GB
  }
}

/**
 * Set cache max bytes and persist to store.
 */
async function setBundleCacheMaxBytes(bytes: number) {
  cacheMaxBytes = Math.max(1, Math.min(bytes, 48 * 1024 * 1024 * 1024))
  const s = await WorkspaceStoreUtils.getStore()
  await s.set('bundle-cache-max-bytes', cacheMaxBytes)
  await s.save()
  cacheUsedBytes = BundleMetaCacheUtils.shrinkToLimit(bundleMetaCache, cacheUsedBytes, cacheMaxBytes)
}

/** Clear all cache */
function clearBundleCache() {
  bundleMetaCache.clear()
  cacheUsedBytes = 0
}

const assetMapState = createWorkspaceAssetMapState()
const {
  assetMapVersion,
  isBundleAssetMapPaged,
  bundleAssetMapTotal,
  bundleAssetClassStats,
  bundleAssetMapLoading,
} = assetMapState

/** Notify AssetMap-backed UI panels that the index state changed. */
function notifyAssetMapChanged() {
  assetMapVersion.value += 1
}

/** Open properties dialog (close context menu) */
function openPropsDialog() {
  // Save selected asset to dialog-specific ref first, since hideContextMenu clears contextMenuAsset
  dialogAsset.value = contextMenuAsset.value
  showPropsDialog.value = true
  hideContextMenu()
}


/** Currently active type filters (empty array = all) */
const activeFilters = ref<string[]>([])

/** File list size sort direction. The first user toggle enters ascending size order. */
const fileSizeSortDirection = ref<SortDirection>('none')

/** Currently active tab in the left panel: files+assets/hierarchy/all-assets */
const activeTab = ref<'files' | 'hierarchy' | 'allassets'>('files')

/** Asset list class name filter (controlled by ClassListPanel, empty string = all) */
const activeClassFilter = ref('')

/** Asset list multi-select type filters for the selected Bundle. */
const activeAssetTypeFilters = ref<string[]>([])

/** Asset list search keyword */
const assetSearchQuery = ref('')

/** Advanced filters for the selected Bundle Asset List. */
const assetAdvancedFilter = ref<AdvancedAssetFilterState>(createDefaultAdvancedAssetFilter())

/**
 * File -> set of asset types contained (for filter highlighting).
 * key: bundle file path, value: set of class_name contained in that bundle
 */
const fileTypeMap = ref<Record<string, Set<string>>>({})

/** Filter class names already resolved from AssetMap index or type scan cache. */
const resolvedFilterTypes = ref<Set<string>>(new Set())

/** List of file paths highlighted under current filter condition (files at the front) */
const highlightedFilePaths = ref<string[]>([])

/** Persistent cache: path -> { md5, types[] }, written to store after each scan */
let persistedTypeCache: Record<string, TypeCacheEntry> = {}

/** Scan concurrency (read from Settings) */
const scanConcurrency = ref(32)

/** Whether type scanning is in progress */
const isTypeScanning = ref(false)

/** Filter is scanning (for file list loading state) */
const isFilterLoading = ref(false)

/** Whether the next active filter change may fall back to a full type scan. */
let allowNextFilterScanFallback = false


/** Currently highlighted index in the left file list (-1 = none) */
const selectedFileIndex = ref(-1)

/** Set of collapsed category names in the asset list */
const collapsedGroups = ref<Set<string>>(new Set())

/** File list container DOM reference, for scrolling into view and receiving keyboard events */
const fileListRef = ref<HTMLElement | null>(null)

/** Asset list container DOM reference, for scrolling into view and receiving keyboard events */
const assetListRef = ref<HTMLElement | null>(null)

/** All Assets list container DOM reference, for scrolling into view and receiving keyboard events */
const allAssetsListRef = ref<HTMLElement | null>(null)

/** Last list panel the user interacted with for document-level arrow navigation. */
const keyboardNavigationScope = ref<KeyboardNavigationScope>('files')

const keyboardNavigators = new Map<KeyboardNavigationScope, KeyboardNavigator>()

/**
 * Number of files to load per chunk.
 * Avoids rendering thousands of files at once, preventing DOM lag.
 */
const FILE_CHUNK_SIZE = 50

/** Number of files currently loaded */
const visibleCount = ref(FILE_CHUNK_SIZE)

/**
 * Number of assets to render per chunk.
 * AssetMap cache can return very large bundles quickly; chunking keeps Vue from mounting
 * the entire asset DOM tree in one frame.
 */
const ASSET_CHUNK_SIZE = 200

/** Number of assets currently rendered in the selected bundle. */
const visibleAssetCount = ref(ASSET_CHUNK_SIZE)

let bundleAssetMapRunId = 0
let bundleAssetFilterTimer: ReturnType<typeof setTimeout> | null = null


const workspaceComputed = WorkspaceComputedUtils.create({
  bundleFiles,
  bundleMeta,
  previewTargetAsset,
  highlightedFilePaths,
  activeClassFilter,
  activeAssetTypeFilters,
  assetSearchQuery,
  assetAdvancedFilter,
  collapsedGroups,
  activeFilters,
  fileSizeSortDirection,
  visibleCount,
  visibleAssetCount,
  isBundleAssetMapPaged,
  bundleAssetMapTotal,
  allAssetTypes: ALL_ASSET_TYPES,
})
const {
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
} = workspaceComputed

const previewStopSignal = useLogSystem().stopSignal

WorkspaceWatchUtils.register({
  activeFilters, highlightedFilePaths, isFilterLoading, resolvedFilterTypes, fileTypeMap,
  bundleMeta, groupedAssets, collapsedGroups, activeClassFilter, activeAssetTypeFilters,
  assetSearchQuery, assetAdvancedFilter,
  persistedTypeCacheRef: { get value() { return persistedTypeCache }, set value(next) { persistedTypeCache = next } },
}, {
  resetFileListChunk, resetAssetListChunk, applyFilters, resolveFiltersFromMapOrScan,
  consumeFilterScanFallback: () => {
    const allowed = allowNextFilterScanFallback
    allowNextFilterScanFallback = false
    return allowed
  },
  t: i18n.global.t,
  addLog: useLogSystem().addLog,
  persistActiveFilters: async () => {
    try {
      const store = await WorkspaceStoreUtils.getStore()
      await store.set('active-filters', activeFilters.value)
      await store.save()
    } catch (error) {
      console.error('Failed to save filter state:', error)
    }
  },
})

watch(previewStopSignal, () => {
  previewLoading.value = false
  const geometry = meshGeometryData.value
  if (geometry) {
    meshGeometryData.value = {
      ...geometry,
      preview_request_key: `${geometry.preview_request_key || ''}:stopped`,
    }
  }
})

watch([activeClassFilter, activeAssetTypeFilters, assetSearchQuery, assetAdvancedFilter], () => {
  if (!isBundleAssetMapPaged.value || !selectedBundlePath.value) return
  if (bundleAssetFilterTimer) clearTimeout(bundleAssetFilterTimer)
  bundleAssetFilterTimer = setTimeout(() => {
    resetAssetListChunk()
    void loadBundleAssetMapPage(true)
  }, 200)
})

watch(activeAssetTypeFilters, (filters) => {
  activeClassFilter.value = filters.length === 1 ? filters[0] : ''
  if (!arraysEqual(assetAdvancedFilter.value.classNames, filters)) {
    assetAdvancedFilter.value = {
      ...assetAdvancedFilter.value,
      classNames: [...filters],
    }
  }
  void persistAssetListFilters()
}, { deep: true })

watch(() => assetAdvancedFilter.value.classNames, (filters) => {
  if (!arraysEqual(activeAssetTypeFilters.value, filters)) {
    activeAssetTypeFilters.value = [...filters]
    activeClassFilter.value = filters.length === 1 ? filters[0] : ''
  }
}, { deep: true })

watch(assetAdvancedFilter, () => {
  void persistAssetListFilters()
}, { deep: true })

/**
 * Load persistent cache + concurrency settings.
 *
 * Read previously scanned type cache (MD5 -> types) from store,
 * and restore fileTypeMap to avoid re-scanning after page navigation.
 */
async function loadTypeScanConfig() {
  const s = await WorkspaceStoreUtils.getStore()
  const saved = await s.get('type-cache')
  if (saved && typeof saved === 'object') {
    persistedTypeCache = saved as Record<string, TypeCacheEntry>
    // Also restore to in-memory fileTypeMap so filters use cache directly,
    // avoiding full re-scan when fileTypeMap is empty
    const restored: Record<string, Set<string>> = {}
    for (const [path, entry] of Object.entries(persistedTypeCache)) {
      if (entry.types.length > 0) {
        restored[path] = new Set(entry.types)
        for (const type of entry.types) {
          resolvedFilterTypes.value.add(type)
        }
      }
    }
    fileTypeMap.value = restored
  }
  const conc = await s.get('scan-concurrency')
  if (typeof conc === 'number' && conc >= 1) {
    scanConcurrency.value = conc
  }
  const savedFileSizeSortDirection = await s.get('file-list-size-sort-direction')
  if (savedFileSizeSortDirection === 'none' || savedFileSizeSortDirection === 'asc' || savedFileSizeSortDirection === 'desc') {
    fileSizeSortDirection.value = savedFileSizeSortDirection
  }
  const savedAssetAdvancedFilter = await s.get('asset-list-advanced-filter')
  if (savedAssetAdvancedFilter && typeof savedAssetAdvancedFilter === 'object') {
    const restoredFilter = normalizeAdvancedAssetFilter(savedAssetAdvancedFilter, { allowBundleFilter: false })
    assetAdvancedFilter.value = restoredFilter
    activeAssetTypeFilters.value = [...restoredFilter.classNames]
    activeClassFilter.value = restoredFilter.classNames.length === 1 ? restoredFilter.classNames[0] : ''
  } else {
    const savedAssetTypeFilters = await s.get('asset-list-active-filters')
    if (Array.isArray(savedAssetTypeFilters)) {
      const restoredFilters = savedAssetTypeFilters.filter((name): name is string => typeof name === 'string' && name.length > 0)
      activeAssetTypeFilters.value = restoredFilters
      activeClassFilter.value = restoredFilters.length === 1 ? restoredFilters[0] : ''
      assetAdvancedFilter.value = {
        ...assetAdvancedFilter.value,
        classNames: [...restoredFilters],
      }
    }
  }
}

async function toggleFileSizeSortDirection() {
  fileSizeSortDirection.value = nextSortDirection(fileSizeSortDirection.value)
  const s = await WorkspaceStoreUtils.getStore()
  await s.set('file-list-size-sort-direction', fileSizeSortDirection.value)
  await s.save()
  resetFileListChunk()
  if (selectedBundlePath.value) {
    selectedFileIndex.value = sortedFiles.value.findIndex(file => file.path === selectedBundlePath.value)
  }
}

function setAssetTypeFilter(className: string) {
  activeAssetTypeFilters.value = className ? [className] : []
  activeClassFilter.value = className
  assetAdvancedFilter.value = {
    ...assetAdvancedFilter.value,
    classNames: [...activeAssetTypeFilters.value],
  }
  void persistAssetListFilters()
}

function clearAssetTypeFilters() {
  activeAssetTypeFilters.value = []
  activeClassFilter.value = ''
  assetAdvancedFilter.value = {
    ...assetAdvancedFilter.value,
    classNames: [],
  }
  void persistAssetListFilters()
}

async function persistAssetListFilters() {
  try {
    const s = await WorkspaceStoreUtils.getStore()
    const normalizedFilter = normalizeAdvancedAssetFilter(assetAdvancedFilter.value, { allowBundleFilter: false })
    await s.set('asset-list-active-filters', normalizedFilter.classNames)
    await s.set('asset-list-advanced-filter', normalizedFilter)
    await s.save()
  } catch (error) {
    console.error('Failed to save Asset List filters:', error)
  }
}

function clearMeshPreviewCacheEntries(bundlePath: string, pathId: string) {
  meshCache.delete(`${bundlePath}::${pathId}`)
  meshCache.delete(WorkspacePreviewUtils.getMeshPreviewCacheKey(bundlePath, pathId))
  meshCache.delete(WorkspacePreviewUtils.getMeshPreviewCacheKey(bundlePath, pathId, WorkspacePreviewUtils.defaultPreviewOptions()))
  for (const key of Array.from(previewAnimatorCache.keys())) {
    if (key.startsWith(`${bundlePath}::${pathId}::animators`)) previewAnimatorCache.delete(key)
  }
  for (const key of Array.from(previewAnimationClipCache.keys())) {
    if (key.startsWith(`${bundlePath}::${pathId}::animation-clips`)) previewAnimationClipCache.delete(key)
  }
}

function formatCacheBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB']
  let value = bytes
  let unitIndex = 0
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024
    unitIndex += 1
  }
  return `${value.toFixed(unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`
}

/** Persist cache to store */
async function persistTypeCache() {
  try {
    const s = await WorkspaceStoreUtils.getStore()
    await s.set('type-cache', persistedTypeCache)
    // Explicitly call save() to ensure immediate write, do not rely on autoSave
    await s.save()
  } catch (e) {
    console.error('Failed to persist type cache:', e)
  }
}

/** Scan mutex, prevents duplicate scanning */
const typeMapScanning = { value: false }

/** Scan type info for all bundles (Rust-side parallel + MD5 cache). */
async function refreshFileTypeMap(forceReScan = false) {
  await WorkspaceTypeScanUtils.refreshFileTypeMap({
    forceReScan,
    scanningRef: typeMapScanning,
    persistedTypeCache,
    state: {
      workDir,
      bundleFiles,
      fileTypeMap,
      resolvedFilterTypes,
      highlightedFilePaths,
      scanConcurrency,
      isTypeScanning,
    },
    persistTypeCache,
    t: i18n.global.t,
    addLog: useLogSystem().addLog,
  })
}

async function resolveFiltersFromMapOrScan(filters: string[], options?: { allowScanFallback?: boolean }) {
  const resolvedFromMap = await WorkspaceTypeScanUtils.resolveFiltersFromMap({
    filters,
    state: {
      workDir,
      bundleFiles,
      fileTypeMap,
      resolvedFilterTypes,
      highlightedFilePaths,
      scanConcurrency,
      isTypeScanning,
    },
    t: i18n.global.t,
    addLog: useLogSystem().addLog,
  })
  if (!resolvedFromMap) {
    if (options?.allowScanFallback) {
      void refreshFileTypeMap(false)
      return
    }
    isFilterLoading.value = false
  }
}

function setActiveFilters(nextFilters: string[], options?: { allowScanFallback?: boolean }) {
  allowNextFilterScanFallback = options?.allowScanFallback === true
  activeFilters.value = [...nextFilters]
}


/** Check if a file contains the current filter type */
function fileMatchesFilter(filePath: string): boolean {
  if (activeFilters.value.length === 0) return false
  const types = fileTypeMap.value[filePath]
  return types ? activeFilters.value.some(filter => types.has(filter)) : false
}

/** List active filters that match a file */
function getFileMatchedFilters(filePath: string): string[] {
  const types = fileTypeMap.value[filePath]
  if (!types || activeFilters.value.length === 0) return []
  return activeFilters.value.filter(filter => types.has(filter))
}

/** Apply the current filter condition, update file list sorting and highlighting. */
function applyFilters(filters: string[]) {
  const totalMatched = WorkspaceTypeScanUtils.applyFilters({
    filters,
    bundleFiles: bundleFiles.value,
    fileTypeMap: fileTypeMap.value,
    highlightedFilePaths,
  })
  if (totalMatched > 0) {
    resetFileListChunk()
    useLogSystem().addLog(i18n.global.t('workspaceLog.filtersMatched', {
      filters: filters.length,
      matched: totalMatched,
      total: bundleFiles.value.length,
    }), 'success')
  }
}

/** Load next batch of files */
function loadMoreFiles() {
  const total = sortedFiles.value.length
  if (visibleCount.value >= total) return
  visibleCount.value = Math.min(visibleCount.value + FILE_CHUNK_SIZE, total)
}

/** Load next batch of assets in the selected Bundle. */
function loadMoreAssets() {
  const total = filteredAssets.value.length
  if (visibleAssetCount.value >= total) return
  visibleAssetCount.value = Math.min(visibleAssetCount.value + ASSET_CHUNK_SIZE, total)
}

/** Scroll detection: auto-load next batch when reaching the bottom */
function onFileListScroll(e: Event) {
  const el = e.target as HTMLElement
  if (!el) return
  const { scrollTop, scrollHeight, clientHeight } = el
  // Trigger load when less than 100px from bottom
  if (scrollHeight - scrollTop - clientHeight < 100) {
    loadMoreFiles()
  }
}

/** Scroll detection for asset list: auto-load next batch near the bottom. */
function onAssetListScroll(e: Event) {
  const el = e.target as HTMLElement
  if (!el) return
  const { scrollTop, scrollHeight, clientHeight } = el
  if (scrollHeight - scrollTop - clientHeight < 160) {
    loadMoreAssets()
  }
}

/** Reset chunk state (called on filter/refresh) */
function resetFileListChunk() {
  visibleCount.value = FILE_CHUNK_SIZE
}

/** Reset asset chunk state (called on bundle/filter/search changes) */
function resetAssetListChunk() {
  visibleAssetCount.value = ASSET_CHUNK_SIZE
}

/** Query the complete AssetMap-backed asset list for the currently selected Bundle. */
async function loadBundleAssetMapPage(reset: boolean): Promise<boolean> {
  const { addLog } = useLogSystem()
  const bundlePath = selectedBundlePath.value
  if (!workDir.value || !bundlePath || bundleAssetMapLoading.value) return false

  const runId = bundleAssetMapRunId
  if (!reset) {
    loadMoreAssets()
    return true
  }

  bundleAssetMapLoading.value = true
  if (reset) {
    bundleAssetMapTotal.value = 0
    bundleMeta.value = workspaceAssetMapClient.createPagedBundleMeta(bundlePath)
  }

  try {
    const result = await workspaceAssetMapClient.loadBundleAssetPage({
      workspaceDir: workDir.value,
      bundlePath,
      search: assetSearchQuery.value,
      classNames: activeAssetTypeFilters.value,
      advancedFilter: assetAdvancedFilter.value,
      offset: 0,
      limit: ASSET_MAP_FULL_QUERY_LIMIT,
    })
    if (runId !== bundleAssetMapRunId || bundlePath !== selectedBundlePath.value || !isBundleAssetMapPaged.value) return false

    bundleMeta.value = workspaceAssetMapClient.applyBundleAssetPage(
      bundleMeta.value,
      bundlePath,
      result.assets,
      reset,
    )
    bundleAssetMapTotal.value = result.total
    resetAssetListChunk()
    return result.total > 0 || result.assets.length > 0
  } catch (error) {
    if (runId === bundleAssetMapRunId) {
      addLog(`AssetMap page query failed, falling back to Bundle parse: ${error}`, 'warn')
    }
    return false
  } finally {
    if (runId === bundleAssetMapRunId) {
      bundleAssetMapLoading.value = false
    }
  }
}

/** Load class counts for the selected Bundle from the AssetMap index. */
async function loadBundleAssetMapClassStats(bundlePath: string, runId: number): Promise<MapAssetClassStat[]> {
  const stats = await workspaceAssetMapClient.loadBundleClassStats(workDir.value, bundlePath)
  if (runId !== bundleAssetMapRunId || bundlePath !== selectedBundlePath.value) return []
  return stats
}

/** Try the AssetMap-backed selected-Bundle list before parsing the Bundle file. */
async function tryLoadBundleFromAssetMap(file: BundleFileEntry): Promise<boolean> {
  const { addLog } = useLogSystem()
  if (!workDir.value) return false
  if (!await workspaceAssetMapClient.hasAssetMap(workDir.value)) return false

  const runId = ++bundleAssetMapRunId
  isBundleAssetMapPaged.value = true
  bundleAssetMapTotal.value = 0
  bundleAssetClassStats.value = []
  bundleAssetMapLoading.value = false
  bundleMeta.value = null
  isLoading.value = true
  loadingText.value = `Loading ${file.name} from AssetMap...`

  try {
    const [stats, hasRows] = await Promise.all([
      loadBundleAssetMapClassStats(file.path, runId),
      loadBundleAssetMapPage(true),
    ])
    if (runId !== bundleAssetMapRunId || file.path !== selectedBundlePath.value) return true
    if (!hasRows) {
      isBundleAssetMapPaged.value = false
      bundleAssetMapTotal.value = 0
      bundleAssetClassStats.value = []
      bundleMeta.value = null
      bundleAssetMapRunId += 1
      return false
    }
    bundleAssetClassStats.value = stats
    const loadedAssets = (bundleMeta.value as BundleMeta | null)?.assets.length || 0
    addLog(`AssetMap hit: ${file.name} (${loadedAssets}/${bundleAssetMapTotal.value} assets loaded)`, 'success')
    isLoading.value = false
    loadingText.value = ''
    return true
  } catch (error) {
    if (runId === bundleAssetMapRunId) {
      addLog(`AssetMap cache unavailable for ${file.name}, parsing Bundle: ${error}`, 'warn')
      isBundleAssetMapPaged.value = false
      bundleAssetMapTotal.value = 0
      bundleAssetClassStats.value = []
      bundleMeta.value = null
      bundleAssetMapRunId += 1
    }
    return false
  } finally {
    if (runId === bundleAssetMapRunId && isBundleAssetMapPaged.value) {
      isLoading.value = false
      loadingText.value = ''
    }
  }
}


/** Refresh button: execute after confirmation */
async function handleRefresh() {
  try {
    await ElMessageBox.confirm(
      i18n.global.t('workspaceLog.refreshConfirmMessage'),
      i18n.global.t('workspaceLog.refreshConfirmTitle'),
      { confirmButtonText: i18n.global.t('workspaceLog.continue'), cancelButtonText: i18n.global.t('common.cancel'), type: 'warning' },
    )
    await loadBundleFiles()
  } catch {
    // User cancelled, do nothing
  }
}

/** Select work directory */
async function selectWorkDir() {
  const { addLog: _addLog } = useLogSystem()
  const selected = await open({
    directory: true,
    multiple: false,
    title: i18n.global.t('workspaceLog.selectWorkDir'),
  })
  if (selected) {
    await switchWorkDir(selected)
  }
}

/**
 * Select one or more Unity file candidates to add to the file list.
 */
async function selectBundleFiles() {
  const { addLog: _addLog } = useLogSystem()
  const selected = await open({
    multiple: true,
    title: i18n.global.t('workspaceLog.selectBundleFiles'),
  })
  if (!selected || selected.length === 0) return
  if (!workDir.value && bundleFiles.value.length === 0) {
    workDir.value = WorkspacePathUtils.parentDirectory(selected[0])
    const s = await WorkspaceStoreUtils.getStore()
    await s.set(LAST_WORKSPACE_DIR_KEY, workDir.value)
    await rememberWorkspaceDir(workDir.value)
  }
  await addBundlePaths(selected)
}

/**
 * Add Unity file candidates from path list (dedup and merge).
 */
async function addBundlePaths(paths: string[]) {
  const { addLog } = useLogSystem()
  if (paths.length === 0) return
  try {
    const newFiles = await invoke<BundleFileEntry[]>('collect_bundle_paths', { paths })
    if (newFiles.length === 0) { addLog(i18n.global.t('workspaceLog.noBundleFiles'), 'warn'); return }
    const existingPaths = new Set(bundleFiles.value.map(f => f.path))
    let added = 0
    for (const f of newFiles) {
      if (!existingPaths.has(f.path)) {
        bundleFiles.value.push(f); existingPaths.add(f.path); added++
      }
    }
    bundleFiles.value.sort((a, b) => a.name.localeCompare(b.name))
    addLog(i18n.global.t('workspaceLog.addedBundleFiles', { added, total: bundleFiles.value.length }), 'success')
    resetFileListChunk()
  } catch (e) { addLog(i18n.global.t('workspaceLog.importFailed', { error: e }), 'error') }
}

function normalizeWorkspaceHistory(value: unknown): string[] {
  if (!Array.isArray(value)) return []
  const seen = new Set<string>()
  const result: string[] = []
  for (const item of value) {
    if (typeof item !== 'string') continue
    const path = item.trim()
    if (!path || seen.has(path)) continue
    seen.add(path)
    result.push(path)
    if (result.length >= MAX_WORKSPACE_HISTORY) break
  }
  return result
}

async function persistWorkspaceHistory() {
  const s = await WorkspaceStoreUtils.getStore()
  await s.set(WORKSPACE_HISTORY_KEY, workspaceHistory.value)
}

async function loadWorkspaceHistory() {
  const s = await WorkspaceStoreUtils.getStore()
  const savedHistory = normalizeWorkspaceHistory(await s.get(WORKSPACE_HISTORY_KEY))
  const savedDir = await s.get(LAST_WORKSPACE_DIR_KEY)
  if (typeof savedDir === 'string' && savedDir.trim()) {
    workspaceHistory.value = rememberWorkspacePath(savedHistory, savedDir)
  } else {
    workspaceHistory.value = savedHistory
  }
  await persistWorkspaceHistory()
}

function rememberWorkspacePath(history: string[], path: string): string[] {
  const normalized = path.trim()
  if (!normalized) return history
  return [
    normalized,
    ...history.filter(item => item !== normalized),
  ].slice(0, MAX_WORKSPACE_HISTORY)
}

async function rememberWorkspaceDir(path: string) {
  workspaceHistory.value = rememberWorkspacePath(workspaceHistory.value, path)
  await persistWorkspaceHistory()
}

function resetWorkspaceState() {
  bundleFileScanRunId += 1
  workDir.value = ''
  bundleFiles.value = []
  selectedBundlePath.value = ''
  bundleMeta.value = null
  selectedAsset.value = null
  previewTargetAsset.value = null
  dumpTargetAsset.value = null
  activeClassFilter.value = ''
  activeAssetTypeFilters.value = []
  assetSearchQuery.value = ''
  assetAdvancedFilter.value = createDefaultAdvancedAssetFilter()
  meshGeometryData.value = null
  previewFilePath.value = ''
  assetProperties.value = {}
  meshCache.clear()
  bundleMetaCache.clear()
  cacheUsedBytes = 0
  isBundleAssetMapPaged.value = false
  bundleAssetMapTotal.value = 0
  bundleAssetClassStats.value = []
  bundleAssetMapLoading.value = false
  bundleAssetMapRunId += 1
  textureCache.clear()
  textureInfo.value = null
  typedPreviewData.value = null
  typedPreviewCache.clear()
  previewAnimators.value = []
  previewAnimatorCache.clear()
  previewAnimationClips.value = []
  previewAnimationClipCache.clear()
  selectedPreviewAnimationClip.value = null
  textureByteSizeCache.clear()
  sceneHierarchy.value = []
  hierarchyLoadedBundlePath = ''
  highlightedFilePaths.value = []
  fileTypeMap.value = {}
  resolvedFilterTypes.value.clear()
  resetFileListChunk()
}

/** Clear workspace */
function clearWorkspace(options?: { log?: boolean }) {
  resetWorkspaceState()
  if (options?.log !== false) {
    useLogSystem().addLog(i18n.global.t('workspaceLog.workspaceCleared'), 'info')
  }
}

async function switchWorkDir(path: string) {
  const selected = path.trim()
  if (!selected) return
  const previous = workDir.value
  if (selected !== previous || bundleFiles.value.length > 0 || selectedBundlePath.value) {
    clearWorkspace({ log: false })
  }
  workDir.value = selected
  const s = await WorkspaceStoreUtils.getStore()
  await s.set(LAST_WORKSPACE_DIR_KEY, selected)
  await rememberWorkspaceDir(selected)
  await loadBundleFiles()
}

async function selectWorkspaceFromHistory(path: string) {
  try {
    await switchWorkDir(path)
  } catch (e) {
    useLogSystem().addLog(i18n.global.t('workspaceLog.scanFailed', { error: e }), 'error')
  }
}

async function restoreWorkspaceDir(path: string) {
  const selected = path.trim()
  if (!selected) return
  workDir.value = selected
  await rememberWorkspaceDir(selected)
}

async function removeCurrentWorkspaceFromHistory() {
  const current = workDir.value
  if (current) {
    workspaceHistory.value = workspaceHistory.value.filter(path => path !== current)
    await persistWorkspaceHistory()
    const s = await WorkspaceStoreUtils.getStore()
    if (await s.get(LAST_WORKSPACE_DIR_KEY) === current) {
      await s.set(LAST_WORKSPACE_DIR_KEY, '')
    }
  }
  clearWorkspace()
}

/** Scan directory and load .bundle file list */
async function loadBundleFiles() {
  const { addLog } = useLogSystem()
  if (!workDir.value) return
  const scanRunId = ++bundleFileScanRunId
  isLoading.value = true
  loadingText.value = i18n.global.t('workspaceLog.scanningFiles')
  addLog(i18n.global.t('workspaceLog.scanningDirectory', { path: workDir.value }), 'info')
  const scanProgressChannel = new Channel<{ step: string; message: string }>()
  let lastScanProgressLog = 0
  scanProgressChannel.onmessage = (payload) => {
    if (scanRunId !== bundleFileScanRunId) return
    loadingText.value = payload.message
    const now = Date.now()
    if (payload.step === 'done' || now - lastScanProgressLog >= 1000) {
      addLog(payload.message, payload.step === 'done' ? 'success' : 'info')
      lastScanProgressLog = now
    }
  }
  try {
    const scannedFiles = await invoke<BundleFileEntry[]>('scan_bundle_files', {
      dir: workDir.value,
      progress: scanProgressChannel,
    })
    if (scanRunId !== bundleFileScanRunId) return
    bundleFiles.value = scannedFiles
    addLog(i18n.global.t('workspaceLog.foundBundleFiles', { count: bundleFiles.value.length }), 'success')
    // File list changed: clean up entries for non-existent files, but keep existing cache
    // This way when the user selects a filter again, it goes directly to cache without re-scanning
    const currentPaths = new Set(bundleFiles.value.map(f => f.path))
    for (const path of Object.keys(fileTypeMap.value)) {
      if (!currentPaths.has(path)) {
        delete fileTypeMap.value[path]
      }
    }
    for (const path of Object.keys(persistedTypeCache)) {
      if (!currentPaths.has(path)) {
        delete persistedTypeCache[path]
      }
    }
    resolvedFilterTypes.value.clear()
    for (const entry of Object.values(persistedTypeCache)) {
      for (const type of entry.types) {
        resolvedFilterTypes.value.add(type)
      }
    }
    // Also clean up persistent cache
    await persistTypeCache()
    // Also clean up BundleMeta memory cache for non-existent files
    for (const cachedPath of bundleMetaCache.keys()) {
      if (!currentPaths.has(cachedPath)) {
        const evicted = bundleMetaCache.get(cachedPath)!
        cacheUsedBytes -= estimateBundleMetaSize(evicted)
        bundleMetaCache.delete(cachedPath)
      }
    }
    highlightedFilePaths.value = []
    isFilterLoading.value = false
    resetFileListChunk()
    if (activeFilters.value.length > 0) setActiveFilters([], { allowScanFallback: false })
  } catch (e) {
    if (scanRunId !== bundleFileScanRunId) return
    addLog(i18n.global.t('workspaceLog.scanFailed', { error: e }), 'error')
    console.error('Failed to scan directory:', e)
    bundleFiles.value = []
  } finally {
    if (scanRunId === bundleFileScanRunId) {
      isLoading.value = false
      loadingText.value = ''
    }
  }
  // Type map is not scanned at startup, it is lazily loaded when the user first selects a filter
}

async function getAssetMapCacheRoot(): Promise<string> {
  return AssetMapCacheUtils.getCacheRoot()
}

async function selectBundle(file: BundleFileEntry) {
  const { addLog, addLogBatch } = useLogSystem()
  // Skip reload if the same file is clicked again
  if (file.path === selectedBundlePath.value && bundleMeta.value) {
    return
  }

  // Update selected index synchronously (for keyboard navigation)
  const idx = sortedFiles.value.findIndex(f => f.path === file.path)
  if (idx >= 0) selectedFileIndex.value = idx

  WorkspacePreviewUtils.trimPreviewCache(meshCache, MESH_PREVIEW_CACHE_LIMIT)
  WorkspacePreviewUtils.trimPreviewCache(textureCache, TEXTURE_PREVIEW_CACHE_LIMIT)
  textureInfo.value = null
  previewAnimators.value = []
  previewAnimationClips.value = []
  selectedPreviewAnimationClip.value = null
  textureByteSizeCache.clear()

  selectedBundlePath.value = file.path
  selectedAsset.value = null
  previewTargetAsset.value = null
  dumpTargetAsset.value = null
  previewFilePath.value = ''
  assetProperties.value = {}
  dumpData.value = null
  dumpError.value = ''
  dumpLoading.value = false
  isBundleAssetMapPaged.value = false
  bundleAssetMapTotal.value = 0
  bundleAssetClassStats.value = []
  resetAssetListChunk()

  if (await tryLoadBundleFromAssetMap(file)) {
    return
  }

  // Check in-memory cache first
  const cached = getCachedBundleMeta(file.path)
  if (cached) {
    isBundleAssetMapPaged.value = false
    bundleMeta.value = cached
    const stats = getBundleCacheStats()
    addLog(i18n.global.t('workspaceLog.cacheHit', {
      name: file.name,
      assets: cached.assets.length,
      entries: stats.entries,
      usedGB: stats.usedGB,
    }), 'success')
    return
  }

  bundleMeta.value = null
  const parseRunId = ++bundleAssetMapRunId
  isBundleAssetMapPaged.value = false
  isLoading.value = true
  loadingText.value = i18n.global.t('workspaceLog.parsingBundleStatus', { name: file.name })
  addLog(i18n.global.t('workspaceLog.parsingBundle', { name: file.name }), 'info')

  // Let the browser render the loading animation first, then start the time-consuming parse
  await nextTick()
  await new Promise(r => requestAnimationFrame(r))

  // Use Channel to receive parse results asynchronously, without blocking UI
  const resultChannel = new Channel<import('../../types').BundleParseChannelResult>()
  resultChannel.onmessage = (message) => {
    if (parseRunId !== bundleAssetMapRunId || selectedBundlePath.value !== file.path) return
    WorkspaceBundleParseUtils.handleParseResult({
      fileName: file.name,
      filePath: file.path,
      message,
      bundleMeta,
      isLoading,
      loadingText,
      setCachedBundleMeta,
      getBundleCacheStats,
      t: i18n.global.t,
      addLog,
      addLogBatch,
    })
  }
  try {
    const assetMapCacheRoot = await getAssetMapCacheRoot()
    await invoke('parse_bundle_metadata', {
      path: file.path,
      onResult: resultChannel,
      workspaceDir: workDir.value || undefined,
      assetMapCacheRoot,
    })
  } catch (e) {
    addLog(i18n.global.t('workspaceLog.parseFailed', { name: file.name, error: e }), 'error')
    console.error('Failed to parse Bundle:', e)
    isLoading.value = false
    loadingText.value = ''
  }
}

/** Get system temp directory as cache */
async function getCacheDir(): Promise<string> {
  if (cacheDir.value) return cacheDir.value
  // Use Tauri's temp directory API, or fall back to .cache/ under work directory
  try {
    const tmp = await tempDir()
    cacheDir.value = tmp + 'assetdaemon-cache'
  } catch {
    cacheDir.value = (workDir.value || '.') + '/.cache'
  }
  return cacheDir.value
}

/**
 * Load scene hierarchy tree.
 */
async function loadSceneHierarchy(bundlePath: string) {
  const { addLog, addLogBatch } = useLogSystem()
  if (hierarchyLoading.value) return
  // Cache hit
  if (hierarchyLoadedBundlePath === bundlePath && sceneHierarchy.value.length > 0) return
  hierarchyLoading.value = true
  sceneHierarchy.value = []
  try {
    const result = await invoke<import('../../types').SceneHierarchyResult>('get_scene_hierarchy', {
      path: bundlePath,
    })
    sceneHierarchy.value = result.roots
    hierarchyLoadedBundlePath = bundlePath
    if (result.diagnostics.length > 0) {
      addLogBatch(result.diagnostics.map(d => ({ message: d, type: 'info' as const })))
    }
    addLog(i18n.global.t('workspaceLog.hierarchyLoaded', { count: result.roots.length }), result.roots.length > 0 ? 'success' : 'warn')
  } catch (e) {
    addLog(i18n.global.t('workspaceLog.hierarchyFailed', { error: e }), 'error')
  } finally {
    hierarchyLoading.value = false
  }
}

/** Select/highlight an asset row without changing the right-side preview or dump target. */
function selectAsset(asset: AssetSummary) {
  selectedAsset.value = asset
}

/** Activate an asset according to the currently selected right-side tab. */
async function activateAsset(asset: AssetSummary) {
  if (rightTab.value === 'dump') {
    await dumpAsset(asset)
    return
  }
  await previewAsset(asset)
}

function setPreviewTargetAsset(asset: AssetSummary) {
  selectedAsset.value = asset
  previewTargetAsset.value = asset
  previewFilePath.value = ''
  previewError.value = ''
  previewLoading.value = false
  meshGeometryData.value = null
  textureInfo.value = null
  typedPreviewData.value = null
  previewAnimators.value = []
  previewAnimationClips.value = []
  selectedPreviewAnimationClip.value = null
  selectedPreviewMeshScope.value = null
  assetProperties.value = WorkspacePreviewUtils.buildBasicAssetProperties(asset)
}

function previewUtilState() {
  return {
    assetProperties,
    previewFilePath,
    previewLoading,
    previewError,
    meshGeometryData,
    textureInfo,
    typedPreviewData,
    previewAnimators,
    previewAnimationClips,
    meshCache,
    textureCache,
    typedPreviewCache,
    previewAnimatorCache,
    previewAnimationClipCache,
    selectedPreviewAnimationClip,
    selectedPreviewMeshScope,
    textureByteSizeCache,
    stopSignal: previewStopSignal,
  }
}

/** Select an asset, extract preview data, and display it on the preview tab. */
async function previewAsset(asset: AssetSummary, previewOptions: AssetPreviewOptions = WorkspacePreviewUtils.defaultPreviewOptions()) {
  const { addLog } = useLogSystem()
  if (asset.class_name === 'Mesh' || asset.class_name === 'Animator' || asset.class_name === 'GameObject') {
    lastModelPreviewOptions.value = { ...previewOptions }
  }
  setPreviewTargetAsset(asset)
  rightTab.value = 'preview'

  const bundlePath = findBundleForAsset(asset)
  const basicProperties = WorkspacePreviewUtils.buildBasicAssetProperties(asset)
  if (!bundlePath) {
    assetProperties.value = basicProperties
    return
  }

  const cache = await getCacheDir()
  await WorkspacePreviewUtils.loadAssetPreview({
    asset,
    bundlePath,
    cacheDir: cache,
    workspaceDirectory: workDir.value,
    previewOptions,
    state: previewUtilState(),
    t: i18n.global.t,
    addLog,
  })
}

/** Lazily scan preview texture candidates for the active model preview. */
async function scanPreviewTextures() {
  const asset = previewTargetAsset.value
  if (!asset) return
  if (asset.class_name !== 'Mesh' && asset.class_name !== 'Animator' && asset.class_name !== 'GameObject') return
  const { addLog } = useLogSystem()
  const bundlePath = findBundleForAsset(asset)
  if (!bundlePath) return
  const cache = await getCacheDir()
  await WorkspacePreviewUtils.scanPreviewTextures({
    asset,
    bundlePath,
    cacheDir: cache,
    workspaceDirectory: workDir.value,
    state: previewUtilState(),
    t: i18n.global.t,
    addLog,
  })
}

/** Lazily scan Animator/Animation component references for the active model preview. */
async function scanPreviewAnimators() {
  const asset = previewTargetAsset.value
  if (!asset) return
  const supported = [
    'Animator',
    'Animation',
    'GameObject',
    'Renderer',
    'MeshRenderer',
    'SkinnedMeshRenderer',
    'MeshFilter',
    'Transform',
  ]
  if (!supported.includes(asset.class_name)) return
  const { addLog } = useLogSystem()
  const bundlePath = findBundleForAsset(asset)
  if (!bundlePath) return
  const assetMapCacheRoot = await getAssetMapCacheRoot()
  const cacheDir = await getCacheDir()
  await WorkspacePreviewUtils.scanPreviewAnimators({
    asset,
    bundlePath,
    workspaceDirectory: workDir.value,
    assetMapCacheRoot,
    cacheDir,
    state: previewUtilState(),
    meshScope: selectedPreviewMeshScope.value,
    addLog,
  })
}

/** Lazily scan related AnimationClip references for the active model preview. */
async function scanPreviewAnimations() {
  const asset = previewTargetAsset.value
  if (!asset) return
  const supported = [
    'Mesh',
    'Animator',
    'Animation',
    'GameObject',
    'Renderer',
    'MeshRenderer',
    'SkinnedMeshRenderer',
    'MeshFilter',
    'Transform',
    'AnimatorController',
    'RuntimeAnimatorController',
    'AnimatorOverrideController',
    'AnimationClip',
  ]
  if (!supported.includes(asset.class_name)) return
  const { addLog } = useLogSystem()
  const bundlePath = findBundleForAsset(asset)
  if (!bundlePath) return
  const assetMapCacheRoot = await getAssetMapCacheRoot()
  const cacheDir = await getCacheDir()
  await WorkspacePreviewUtils.scanPreviewAnimations({
    asset,
    bundlePath,
    workspaceDirectory: workDir.value,
    assetMapCacheRoot,
    cacheDir,
    state: previewUtilState(),
    meshScope: selectedPreviewMeshScope.value,
    addLog,
  })
}

/** Parse and apply one selected AnimationClip to the current model preview. */
async function applyPreviewAnimationClip(clip: import('../../types').PreviewAnimationClipRef) {
  const asset = previewTargetAsset.value
  if (!asset) return
  if (!previewAnimators.value.length) {
    useLogSystem().addLog('  -> Scan Animator before applying an AnimationClip', 'warn')
    return
  }
  const bundlePath = findBundleForAsset(asset)
  if (!bundlePath) return
  const cache = await getCacheDir()
  await WorkspacePreviewUtils.applyPreviewAnimationClip({
    clip,
    asset,
    bundlePath,
    cacheDir: cache,
    workspaceDirectory: workDir.value,
    state: previewUtilState(),
    t: i18n.global.t,
    addLog: useLogSystem().addLog,
  })
}


/** Select an asset, switch to the dump tab, and fetch dump data. */
async function dumpAsset(asset: AssetSummary) {
  selectedAsset.value = asset
  dumpTargetAsset.value = asset
  dumpData.value = null
  dumpError.value = ''
  dumpLoading.value = false
  assetProperties.value = WorkspacePreviewUtils.buildBasicAssetProperties(asset)
  rightTab.value = 'dump'
  await fetchDumpData()
}


/**
 * Find the bundle file path for the given asset.
 * Prioritizes the currently selected file, then searches all cached bundles.
 */
function findBundleForAsset(asset: AssetSummary): string | null {
  if (asset.source_bundle_path) {
    return asset.source_bundle_path
  }

  const curPath = selectedBundlePath.value
  if (curPath) {
    const meta = getCachedBundleMeta(curPath)
    if (meta?.assets?.some(a => a.path_id === asset.path_id)) {
      return curPath
    }
  }
  for (const [path, meta] of bundleMetaCache.entries()) {
    if (meta.assets?.some(a => a.path_id === asset.path_id)) {
      return path
    }
  }
  return null
}

/**
 * Get dump data for the current dump target asset (with cache).
 *
 * - First check dumpCache, if hit set directly to dumpData
 * - On miss, call Rust backend to extract, write to cache on success
 * - Concurrent protection: skip if already loading
 */
async function fetchDumpData() {
  await WorkspaceDumpUtils.fetchDumpData({
    dumpTargetAsset,
    dumpLoading,
    dumpError,
    dumpData,
    dumpCache,
    findBundleForAsset,
    t: i18n.global.t,
    addLog: useLogSystem().addLog,
  })
}

/** Keyboard navigation: up/down arrow to switch files */
function handleFileListKeydown(e: KeyboardEvent): boolean {
  const files = sortedFiles.value
  if (files.length === 0) return false

  if (e.key === 'ArrowDown') {
    e.preventDefault()
    const next = selectedFileIndex.value < files.length - 1
      ? selectedFileIndex.value + 1
      : 0 // Wrap to top when reaching the end
    selectFileByIndex(next)
    return true
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    const prev = selectedFileIndex.value > 0
      ? selectedFileIndex.value - 1
      : files.length - 1 // Wrap to bottom when reaching the top
    selectFileByIndex(prev)
    return true
  }
  return false
}

/** Select file by index and auto-scroll into view */
async function selectFileByIndex(index: number) {
  const files = sortedFiles.value
  if (index < 0 || index >= files.length) return

  selectedFileIndex.value = index
  await selectBundle(files[index])

  // After DOM update, scroll the selected item into view
  await nextTick()
  const container = fileListRef.value
  if (!container) return
  const items = container.querySelectorAll('.file-item')
  const target = items[index] as HTMLElement | undefined
  if (target) {
    target.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
  }
}

function setKeyboardNavigationScope(scope: KeyboardNavigationScope) {
  keyboardNavigationScope.value = scope
}

function registerKeyboardNavigator(scope: KeyboardNavigationScope, handler: KeyboardNavigator) {
  keyboardNavigators.set(scope, handler)
  return () => {
    if (keyboardNavigators.get(scope) === handler) {
      keyboardNavigators.delete(scope)
    }
  }
}

function shouldIgnoreListNavigation(e: KeyboardEvent): boolean {
  if (e.defaultPrevented || e.isComposing || e.altKey || e.ctrlKey || e.metaKey) return true
  if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return true

  const active = document.activeElement instanceof HTMLElement ? document.activeElement : null
  const target = e.target instanceof HTMLElement ? e.target : active
  const element = target || active
  if (!element) return false

  return Boolean(element.closest([
    'input',
    'textarea',
    'select',
    '[contenteditable="true"]',
    '.el-input',
    '.el-input-tag',
    '.el-select',
    '.el-overlay',
    '.el-popper',
    '.el-dialog',
    '.file-context-menu',
    '.asset-context-menu',
    '.preview-context-menu',
  ].join(',')))
}

/**
 * Global keyboard event handler.
 * Binds keyboard listener to document so arrow keys switch rows instead of scrolling the list.
 */
function onKeydown(e: KeyboardEvent) {
  if (shouldIgnoreListNavigation(e)) return

  const active = document.activeElement
  if (active instanceof Node) {
    if (fileListRef.value?.contains(active)) {
      keyboardNavigationScope.value = 'files'
    } else if (assetListRef.value?.contains(active)) {
      keyboardNavigationScope.value = 'bundle-assets'
    } else if (allAssetsListRef.value?.contains(active)) {
      keyboardNavigationScope.value = 'all-assets'
    }
  }

  const handler = keyboardNavigators.get(keyboardNavigationScope.value)
    || (keyboardNavigationScope.value === 'files' ? handleFileListKeydown : undefined)
  if (handler?.(e)) {
    e.preventDefault()
  }
}


/** Toggle collapse state of an asset category */
function toggleGroup(className: string) {
  const s = new Set(collapsedGroups.value)
  if (s.has(className)) {
    s.delete(className)
  } else {
    s.add(className)
  }
  collapsedGroups.value = s
}



/** Show context menu */
function showContextMenu(e: MouseEvent, asset: AssetSummary) {
  e.preventDefault()
  contextMenuVisible.value = true
  contextMenuX.value = e.clientX
  contextMenuY.value = e.clientY
  contextMenuAsset.value = asset
}

/** Hide context menu */
function hideContextMenu() {
  contextMenuVisible.value = false
  contextMenuAsset.value = null
}

/** Clear single asset's Mesh cache (force re-parse) */
async function clearSingleMeshCache() {
  const asset = contextMenuAsset.value
  const bundlePath = asset?.source_bundle_path || selectedBundlePath.value
  if (asset && bundlePath) {
    const { addLog } = useLogSystem()
    clearMeshPreviewCacheEntries(bundlePath, asset.path_id)
    try {
      const dir = await getCacheDir()
      const result = await invoke<ClearPreviewCacheResult>('clear_asset_preview_cache', {
        bundlePath,
        pathId: asset.path_id,
        cacheDir: dir,
      })
      addLog(
        `${i18n.global.t('workspaceLog.clearedSingleCache', { name: AssetDisplayUtils.getAssetDisplayName(asset) })} (disk preview paths: ${result.removed_paths}, ${formatCacheBytes(result.removed_bytes)})`,
        'info',
      )
    } catch (e) {
      addLog(`Failed to clear disk preview cache: ${e}`, 'warn')
      addLog(i18n.global.t('workspaceLog.clearedSingleCache', { name: AssetDisplayUtils.getAssetDisplayName(asset) }), 'info')
    }
  }
  hideContextMenu()
}

/** Get display size of an asset, preferring decoded texture byte size when available. */
const getAssetSize = (asset: {
  class_name: string
  path_id: string
  byte_size: number
  source_bundle_path?: string
}) =>
  WorkspacePreviewUtils.getAssetSize(
    asset,
    textureByteSizeCache,
    asset.source_bundle_path || selectedBundlePath.value,
  )

/** Clear all caches (Mesh + Texture2D) */
async function clearAllMeshCache() {
  const meshCount = meshCache.size
  const txCount = textureCache.size
  meshCache.clear()
  textureCache.clear()
  previewAnimators.value = []
  previewAnimatorCache.clear()
  previewAnimationClips.value = []
  previewAnimationClipCache.clear()
  selectedPreviewAnimationClip.value = null
  selectedPreviewMeshScope.value = null
  textureByteSizeCache.clear()
  const { addLog } = useLogSystem()
  try {
    const dir = await getCacheDir()
    const result = await invoke<ClearPreviewCacheResult>('clear_all_preview_cache', { cacheDir: dir })
    addLog(
      `${i18n.global.t('workspaceLog.clearedAllCaches', { mesh: meshCount, textures: txCount })} (disk preview paths: ${result.removed_paths}, ${formatCacheBytes(result.removed_bytes)})`,
      'info',
    )
  } catch (e) {
    addLog(`Failed to clear disk preview cache: ${e}`, 'warn')
    addLog(i18n.global.t('workspaceLog.clearedAllCaches', { mesh: meshCount, textures: txCount }), 'info')
  }
  hideContextMenu()
}

type PreviewDrawerCacheCategory = 'mesh' | 'texture' | 'animator' | 'animation'

function clearPreviewDrawerCache(category: PreviewDrawerCacheCategory) {
  const { addLog } = useLogSystem()
  if (category === 'mesh') {
    const count = meshCache.size
    meshCache.clear()
    meshGeometryData.value = null
    selectedPreviewMeshScope.value = null
    previewAnimators.value = []
    previewAnimationClips.value = []
    selectedPreviewAnimationClip.value = null
    addLog(`Cleared Mesh preview cache (${count} entr${count === 1 ? 'y' : 'ies'})`, 'info')
    return
  }

  if (category === 'texture') {
    const count = textureCache.size
    textureCache.clear()
    textureByteSizeCache.clear()
    textureInfo.value = null
    if (meshGeometryData.value?.texture_candidates?.length) {
      meshGeometryData.value = {
        ...meshGeometryData.value,
        texture_candidates: [],
        glb_texture_count: 0,
        diffuse_texture_path: undefined,
        diffuse_texture_path_id: undefined,
        diffuse_texture_bundle_path: undefined,
        diffuse_texture_name: undefined,
        diffuse_material_name: undefined,
        diffuse_slot_name: undefined,
        diffuse_texture_user_selected: false,
      }
    }
    addLog(`Cleared Texture preview cache (${count} entr${count === 1 ? 'y' : 'ies'})`, 'info')
    return
  }

  if (category === 'animator') {
    const count = previewAnimatorCache.size
    previewAnimatorCache.clear()
    previewAnimators.value = []
    previewAnimationClipCache.clear()
    previewAnimationClips.value = []
    selectedPreviewAnimationClip.value = null
    addLog(`Cleared Animator preview cache (${count} entr${count === 1 ? 'y' : 'ies'})`, 'info')
    return
  }

  const count = previewAnimationClipCache.size
  previewAnimationClipCache.clear()
  previewAnimationClips.value = []
  selectedPreviewAnimationClip.value = null
  if (meshGeometryData.value) {
    meshGeometryData.value = {
      ...meshGeometryData.value,
      glb_animation_count: 0,
      glb_animation_names: [],
    }
  }
  addLog(`Cleared AnimationClip preview cache (${count} entr${count === 1 ? 'y' : 'ies'})`, 'info')
}


export function useWorkspace() {
  return WorkspaceReturnUtils.build({
    workDir, bundleFiles, selectedBundlePath, bundleMeta, isLoading, loadingText,
    lastModelPreviewOptions,
    selectedAsset, previewTargetAsset, dumpTargetAsset, assetProperties, previewFilePath, textureInfo, typedPreviewData, previewAnimators, previewAnimationClips, selectedPreviewAnimationClip, selectedPreviewMeshScope, textureByteSizeCache,
    meshGeometryData, meshCache, previewLoading, previewError, cacheDir,
    contextMenuVisible, contextMenuX, contextMenuY, contextMenuAsset, dialogAsset, showPropsDialog,
    openPropsDialog, showContextMenu, hideContextMenu, clearSingleMeshCache, clearAllMeshCache, clearPreviewDrawerCache,
    getAssetSize, getAllCachedAssets, bundleMetaCache,
    activeFilters, setActiveFilters, fileSizeSortDirection, toggleFileSizeSortDirection,
    activeClassFilter, activeAssetTypeFilters, setAssetTypeFilter, clearAssetTypeFilters,
    assetSearchQuery, assetAdvancedFilter, activeTab, fileTypeMap,
    assetMapVersion, notifyAssetMapChanged,
    highlightedFilePaths, isTypeScanning, isFilterLoading, scanConcurrency,
    selectedFileIndex, collapsedGroups, fileListRef, assetListRef, allAssetsListRef, visibleCount,
    keyboardNavigationScope, setKeyboardNavigationScope, registerKeyboardNavigator,
    visibleAssetCount, isBundleAssetMapPaged, bundleAssetMapTotal,
    bundleAssetClassStats, bundleAssetMapLoading,
    sortedFiles, visibleFiles, hasMoreFiles, allClassNames, groupedAssets, filteredAssets,
    visibleAssets, hasMoreAssets,
    filteredGroupedAssets, filteredAssetGroupCounts, filteredAssetClassNames, assetClassNames, previewType,
    loadTypeScanConfig, refreshFileTypeMap, persistTypeCache, fileMatchesFilter,
    getFileMatchedFilters, applyFilters,
    loadMoreFiles, onFileListScroll, resetFileListChunk,
    loadMoreAssets, onAssetListScroll, resetAssetListChunk,
    workspaceHistory, loadWorkspaceHistory, selectWorkspaceFromHistory, restoreWorkspaceDir,
    handleRefresh, selectWorkDir, selectBundleFiles, addBundlePaths, clearWorkspace, removeCurrentWorkspaceFromHistory,
    loadBundleFiles, selectBundle, selectAsset, activateAsset, previewAsset, scanPreviewTextures, scanPreviewAnimators, scanPreviewAnimations, applyPreviewAnimationClip, dumpAsset, getCacheDir, sceneHierarchy, hierarchyLoading,
    loadSceneHierarchy, rightTab, dumpData, dumpLoading, dumpError, dumpCache,
    fetchDumpData, findBundleForAsset, handleFileListKeydown, selectFileByIndex, onKeydown,
    toggleGroup, getStore: WorkspaceStoreUtils.getStore, loadBundleCacheConfig, setBundleCacheMaxBytes,
    getAssetMapCacheRoot,
    getBundleCacheStats, clearBundleCache,
  })
}



