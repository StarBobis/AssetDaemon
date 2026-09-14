<script setup lang="ts">
/**
 * AssetListPanel ->Bottom-left section: Asset list
 *
 * Supports:
 * - Class name filtering (set by ClassListPanel via activeClassFilter)
 * - Search keyword filtering (real-time search by name/path/class_name)
 * - Grouped display by class name, collapse/expand, selection highlight
 * - Right-click clear cache
 */

import { onMounted, onUnmounted, watch, ref, nextTick, computed } from 'vue'
import { Search, Close, Refresh } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import { useWorkspace } from '../../composables/home/useWorkspace'
import { useExport } from '../../composables/home/useExport'
import { AssetDisplayUtils } from '../../utils/AssetDisplayUtils'
import { ClassIconUtils } from '../../utils/ClassIconUtils'
import AssetContextMenu from './AssetContextMenu.vue'
import AssetListItem from './AssetListItem.vue'
import AdvancedAssetFilterCard from './AdvancedAssetFilterCard.vue'
import type { AdvancedAssetFilterState, AssetSummary, ExportAssetRef } from '../../types'
import { createDefaultAdvancedAssetFilter, normalizeAdvancedAssetFilter } from '../../utils/AdvancedAssetFilterUtils'

const ws = useWorkspace()
const exp = useExport()
const { t } = useI18n()

/** Search input ref, for auto-focus */
const searchInputRef = ref<HTMLInputElement | null>(null)
const advancedFilterVisible = ref(false)
let unregisterAssetKeyboardNavigator: (() => void) | null = null
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

function setAssetListElement(element: unknown) {
  ws.assetListRef.value = element instanceof HTMLElement ? element : null
}

const typeStats = computed(() => {
  return ws.bundleAssetClassStats.value.length > 0
    ? ws.bundleAssetClassStats.value
    : ws.assetClassNames.value.map(name => ({
      name,
      count: ws.groupedAssets.value[name]?.length || 0,
    }))
})

const advancedFilterActiveCount = computed(() => {
  const filter = ws.assetAdvancedFilter.value
  let count = filter.classNames.length
  count += normalizedTextList([
    filter.nameQuery,
    ...filter.nameIncludeQueries,
  ]).length
  count += normalizedTextList([
    filter.nameExcludeQuery,
    ...filter.nameExcludeQueries,
  ]).length
  if (filter.pathIdQuery.trim()) count += 1
  if (filter.minSizeText.trim()) count += 1
  if (filter.maxSizeText.trim()) count += 1
  if (filter.sortBy !== 'type' || filter.sortDirection !== 'asc') count += 1
  return count
})

/** Clear class name filter */
function clearClassFilter() {
  ws.clearAssetTypeFilters()
}

/** Clear search keyword */
function clearSearch() {
  ws.assetSearchQuery.value = ''
  nextTick(() => searchInputRef.value?.focus())
}

function normalizedTextList(values: Array<string | undefined>): string[] {
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

function applyAdvancedFilter(filter: AdvancedAssetFilterState) {
  ws.assetAdvancedFilter.value = normalizeAdvancedAssetFilter(filter, { allowBundleFilter: false })
  ws.activeAssetTypeFilters.value = [...ws.assetAdvancedFilter.value.classNames]
}

function resetAdvancedFilter() {
  ws.assetAdvancedFilter.value = createDefaultAdvancedAssetFilter()
  ws.activeAssetTypeFilters.value = []
  ws.activeClassFilter.value = ''
}

function assetBundlePath(asset: AssetSummary): string {
  return asset.source_bundle_path || ws.selectedBundlePath.value
}

function assetSelectionKey(asset: AssetSummary): string {
  return `${assetBundlePath(asset)}::${asset.path_id}`
}

function exportRefForAsset(asset: AssetSummary): ExportAssetRef {
  return {
    bundle_path: assetBundlePath(asset),
    path_id: asset.path_id,
    class_name: asset.class_name,
    asset_name: AssetDisplayUtils.getAssetExportName(asset),
  }
}

function selectedRefKey(ref: ExportAssetRef): string {
  return `${ref.bundle_path}::${ref.path_id}`
}

const selectedAssetKeys = computed(() => new Set(exp.selectedAssets.value.map(selectedRefKey)))

/** Check if asset is in the multi-select list */
function isAssetSelected(asset: AssetSummary): boolean {
  return selectedAssetKeys.value.has(assetSelectionKey(asset))
}

/** Clear multi-selection */
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
  const key = assetSelectionKey(asset)
  if (selectedAssetKeys.value.has(key)) {
    setSelectedAssetRefs(exp.selectedAssets.value.filter(ref => selectedRefKey(ref) !== key))
  } else {
    addSelectedAssets([asset])
  }
  lastSelectionAnchorKey.value = key
}

function selectAssetRange(targetAsset: AssetSummary) {
  const list = navigableAssets.value
  const targetKey = assetSelectionKey(targetAsset)
  const anchorKey = lastSelectionAnchorKey.value || targetKey
  const anchorIndex = list.findIndex(asset => assetSelectionKey(asset) === anchorKey)
  const targetIndex = list.findIndex(asset => assetSelectionKey(asset) === targetKey)
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
  activateListAsset(asset)
}

function handleCheckboxClick(event: MouseEvent, asset: AssetSummary) {
  event.stopPropagation()
  if (event.shiftKey) {
    selectAssetRange(asset)
    return
  }
  toggleAssetSelection(asset)
}

function assetKey(asset: AssetSummary): string {
  return `${asset.source_bundle_path || ws.selectedBundlePath.value}::${asset.path_id}`
}

function assetFullDisplayPath(asset: AssetSummary): string {
  return asset.source_bundle_path || ws.selectedBundlePath.value
}

function assetHierarchyPath(asset: AssetSummary): string {
  return asset.path || ''
}

const navigableAssets = computed(() => {
  const visibleClassNames = ws.filteredAssetClassNames.value.filter(className => !ws.collapsedGroups.value.has(className))
  const grouped: Record<string, AssetSummary[]> = {}
  for (const asset of ws.filteredAssets.value) {
    if (!grouped[asset.class_name]) grouped[asset.class_name] = []
    grouped[asset.class_name].push(asset)
  }
  return visibleClassNames.flatMap(className => grouped[className] || [])
})

function selectedAssetIndex(): number {
  const selected = ws.selectedAsset.value
  if (!selected) return -1
  const selectedKey = assetKey(selected)
  return navigableAssets.value.findIndex(asset => assetKey(asset) === selectedKey)
}

async function ensureAssetVisible(index: number) {
  while (index >= visibleNavigableAssets().length && ws.hasMoreAssets.value) {
    const before = ws.visibleAssetCount.value
    ws.loadMoreAssets()
    await nextTick()
    if (ws.visibleAssetCount.value <= before) break
  }
}

function visibleNavigableAssets(): AssetSummary[] {
  const visibleKeys = new Set(ws.visibleAssets.value.map(asset => assetKey(asset)))
  return navigableAssets.value.filter(asset => visibleKeys.has(assetKey(asset)))
}

async function selectAssetByIndex(index: number) {
  const list = navigableAssets.value
  if (index < 0 || index >= list.length) return
  ws.setKeyboardNavigationScope('bundle-assets')
  const asset = list[index]
  await ensureAssetVisible(index)
  await ws.activateAsset(asset)
  await nextTick()
  const container = ws.assetListRef.value
  const target = container?.querySelector(`[data-asset-key="${CSS.escape(assetKey(asset))}"]`) as HTMLElement | null
  target?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
}

function handleAssetListKeydown(event: KeyboardEvent): boolean {
  if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return false
  const list = navigableAssets.value
  if (list.length === 0) return false
  event.preventDefault()
  const currentIndex = selectedAssetIndex()
  const nextIndex = event.key === 'ArrowDown'
    ? (currentIndex >= 0 && currentIndex < list.length - 1 ? currentIndex + 1 : 0)
    : (currentIndex > 0 ? currentIndex - 1 : list.length - 1)
  void selectAssetByIndex(nextIndex)
  return true
}

function activateListAsset(asset: AssetSummary) {
  ws.setKeyboardNavigationScope('bundle-assets')
  ws.activateAsset(asset)
}

function shouldIgnoreSelectionDrag(event: PointerEvent): boolean {
  const target = event.target as HTMLElement | null
  if (!target) return true
  return Boolean(target.closest([
    '.asset-checkbox',
    '.asset-group-title',
    'button',
    'input',
    'textarea',
    'select',
    '.el-input',
    '.el-checkbox',
    '.el-button',
    '.el-popper',
  ].join(',')))
}

function handleAssetListPointerDown(event: PointerEvent) {
  ws.setKeyboardNavigationScope('bundle-assets')
  suppressNextAssetClick = false
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
  const selectedKeys = new Set<string>()
  ws.assetListRef.value
    ?.querySelectorAll<HTMLElement>('[data-asset-key]')
    .forEach((element) => {
      const rect = element.getBoundingClientRect()
      if (rectIntersects(selectionRect, rect)) {
        const key = element.dataset.assetKey
        if (key) selectedKeys.add(key)
      }
    })
  if (selectedKeys.size === 0) return

  const selectedByDrag = navigableAssets.value.filter(asset => selectedKeys.has(assetSelectionKey(asset)))
  if (drag.additive) {
    addSelectedAssets(selectedByDrag)
  } else {
    setSelectedAssets(selectedByDrag)
  }
  if (selectedByDrag.length === 0) return
  lastSelectionAnchorKey.value = assetSelectionKey(selectedByDrag[selectedByDrag.length - 1])
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

onMounted(() => {
  unregisterAssetKeyboardNavigator = ws.registerKeyboardNavigator('bundle-assets', handleAssetListKeydown)
})

onUnmounted(() => {
  document.removeEventListener('pointermove', handleSelectionPointerMove)
  unregisterAssetKeyboardNavigator?.()
  unregisterAssetKeyboardNavigator = null
})

/** Clear multi-selection when switching bundles */
watch(() => ws.selectedBundlePath.value, () => {
  exp.selectedAssets.value = []
})

/** When type filters change, auto-expand selected categories and collapse others */
watch(() => ws.activeAssetTypeFilters.value, (filters) => {
  if (filters.length === 0) return
  const filterSet = new Set(filters)
  const allNames = new Set(Object.keys(ws.filteredGroupedAssets.value))
  for (const name of allNames) {
    if (!filterSet.has(name)) {
      ws.collapsedGroups.value = new Set([...ws.collapsedGroups.value, name])
    } else {
      const s = new Set(ws.collapsedGroups.value)
      s.delete(name)
      ws.collapsedGroups.value = s
    }
  }
})
</script>

<template>
  <template v-if="ws.bundleMeta.value">
    <!-- Group list header: search box + counter -->
    <div class="pane-header">
      <span class="pane-header-left">
        {{ t('home.assetList') }}
        <span class="filter-status" v-if="ws.activeAssetTypeFilters.value.length || ws.assetSearchQuery.value">
          <span class="filter-badge" v-if="ws.activeAssetTypeFilters.value.length">
            {{ ws.activeAssetTypeFilters.value.join(', ') }}
            <el-icon class="filter-clear" @click.stop="clearClassFilter"><Close /></el-icon>
          </span>
          <span class="filter-badge search-badge" v-if="ws.assetSearchQuery.value">
            "{{ ws.assetSearchQuery.value }}"
            <el-icon class="filter-clear" @click.stop="clearSearch"><Close /></el-icon>
          </span>
        </span>
      </span>
      <span class="pane-header-right">
        <AdvancedAssetFilterCard
          v-model:visible="advancedFilterVisible"
          v-model="ws.assetAdvancedFilter.value"
          :type-stats="typeStats"
          :disabled="!ws.bundleMeta.value || ws.bundleAssetMapLoading.value"
          :active-count="advancedFilterActiveCount"
          :show-bundle-filter="false"
          @apply="applyAdvancedFilter"
          @reset="resetAdvancedFilter"
        />
      </span>
      <span class="panel-count">
        {{ ws.visibleAssets.value.length }}/{{ ws.filteredAssets.value.length }}
      </span>
    </div>

    <!-- Search box -->
    <div class="asset-search-bar">
      <el-input
        v-model="ws.assetSearchQuery.value"
        :placeholder="t('home.searchAssetPlaceholder')"
        size="small"
        clearable
        :prefix-icon="Search"
        class="asset-search-input"
        ref="searchInputRef"
      />
    </div>

    <!-- Multi-select status bar -->
    <div v-if="exp.selectedAssets.value.length > 0" class="selection-bar">
      <span class="selection-text">
        {{ t('home.selectedCount', { count: exp.selectedAssets.value.length }) }}
      </span>
      <span class="selection-hint">{{ t('home.multiSelectHint') }}</span>
      <el-button size="small" text type="danger" @click="clearSelection()">{{ t('home.clearSelection') }}</el-button>
    </div>

    <!-- Asset list scroll container -->
    <div
      :ref="setAssetListElement"
      class="pane-scroll pane-assets"
      tabindex="0"
      @focusin="ws.setKeyboardNavigationScope('bundle-assets')"
      @pointerdown="handleAssetListPointerDown"
      @keydown="handleAssetListKeydown"
      @scroll="ws.onAssetListScroll"
    >
      <!-- No matches after filtering -->
      <div v-if="!ws.bundleAssetMapLoading.value && ws.filteredAssets.value.length === 0" class="panel-empty">
        {{ t('common.noMatchingAssets') }}
      </div>

      <!-- Grouped by class name -->
      <div
        v-for="className in ws.filteredAssetClassNames.value"
        :key="className"
        class="asset-group"
      >
        <!-- Group title (clickable to collapse) -->
        <div
          class="asset-group-title"
          :class="{ 'asset-group-collapsed': ws.collapsedGroups.value.has(className) }"
          @click="ws.toggleGroup(className)"
        >
          <span class="asset-group-arrow">{{ ws.collapsedGroups.value.has(className) ? '+' : '-' }}</span>
          <el-icon :size="16"><component :is="ClassIconUtils.getClassIcon(className)" /></el-icon>
          {{ className }}
          <span class="asset-group-count">
            ({{ ws.filteredAssetGroupCounts.value[className] || 0 }})
          </span>
        </div>

        <!-- Group asset list -->
        <AssetListItem
          v-for="asset in ws.filteredGroupedAssets.value[className]"
          :key="asset.path_id"
          v-show="!ws.collapsedGroups.value.has(className)"
          :asset="asset"
          :asset-key="assetKey(asset)"
          :active="ws.selectedAsset.value?.path_id === asset.path_id"
          :selected="isAssetSelected(asset)"
          :selectable="true"
          :bundle-path="assetFullDisplayPath(asset)"
          :hierarchy-path="assetHierarchyPath(asset)"
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

    <AssetContextMenu :assets="ws.bundleMeta.value?.assets || []" />
  </template>

  <!-- Empty state / loading when no bundle selected -->
  <div v-else class="pane-scroll pane-assets pane-assets-empty">
    <!-- Parsing bundle -->
    <div v-if="ws.isLoading.value" class="panel-loading">
      <el-icon class="is-loading" :size="32"><Refresh /></el-icon>
      <p>{{ ws.loadingText.value || t('home.parsing') }}</p>
    </div>
    <!-- Completely empty state -->
    <div v-else class="panel-empty">
      {{ t('home.clickToView') }}
    </div>
  </div>

</template>

<style scoped>
/*
 * Asset list panel styles
 * Most styles are shared with FileListPanel, see global CSS definitions in Home.vue
 */

/* ---- Search box ---- */
.asset-search-bar {
  flex-shrink: 0;
  padding: 8px;
  background: var(--app-surface);
  border-bottom: 1px solid var(--app-border-soft);
}

.asset-search-input {
  width: 100%;
}

.asset-selection-box {
  position: fixed;
  z-index: 9999;
  pointer-events: none;
  border: 1px solid var(--el-color-primary);
  background: color-mix(in srgb, var(--el-color-primary) 16%, transparent);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--el-color-primary) 18%, transparent) inset;
}

/* ---- Multi-select status bar ---- */
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

/* ---- Filter status labels ---- */
.pane-header-left {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  min-width: 0;
  overflow: hidden;
}

.filter-status {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: 1;
  min-width: 0;
  overflow: hidden;
}

.filter-badge {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 0 6px;
  font-size: 11px;
  font-weight: 500;
  line-height: 18px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--el-color-primary) 12%, transparent);
  color: var(--el-color-primary);
  text-transform: none;
  letter-spacing: 0;
  white-space: nowrap;
  max-width: 150px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.search-badge {
  background: color-mix(in srgb, var(--el-color-warning) 14%, transparent);
  color: var(--el-color-warning);
}

.filter-clear {
  cursor: pointer;
  font-size: 12px;
  flex-shrink: 0;
}

.filter-clear:hover {
  color: var(--el-color-danger);
}

/* Context menu styles moved to Home.vue global CSS */
</style>
