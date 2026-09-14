<script setup lang="ts">
/**
 * SearchPanel ->Global asset search panel (Tab 5)
 *
 * Search box to search all assets in the current bundle by name/path/type.
 * Results shown instantly while typing, supports keyboard navigation.
 */

import { ref, computed, watch, nextTick } from 'vue'
import { Search, Close } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import { useWorkspace } from '../../composables/home/useWorkspace'
import { ClassIconUtils } from '../../utils/ClassIconUtils'
import { AssetDisplayUtils } from '../../utils/AssetDisplayUtils'
import type { AssetSummary } from '../../types'

const ws = useWorkspace()
const { t } = useI18n()

/** Local search keyword */
const query = ref('')

/** Search result highlight index (keyboard up/down) */
const highlightIndex = ref(-1)

/** Search result list ref for scrolling */
const resultListRef = ref<HTMLDivElement | null>(null)

/** Search results: filtered from bundleMeta */
const results = computed(() => {
  const q = query.value.trim().toLowerCase()
  if (!q || !ws.bundleMeta.value?.assets) return []

  return ws.bundleMeta.value.assets.filter(a =>
    AssetDisplayUtils.getAssetDisplayName(a).toLowerCase().includes(q) ||
    a.name.toLowerCase().includes(q) ||
    a.path.toLowerCase().includes(q) ||
    a.class_name.toLowerCase().includes(q)
  )
})

/** Search results grouped by type */
const groupedResults = computed(() => {
  const groups: Record<string, AssetSummary[]> = {}
  for (const asset of results.value) {
    if (!groups[asset.class_name]) groups[asset.class_name] = []
    groups[asset.class_name].push(asset)
  }
  return groups
})

/** Group name list */
const groupNames = computed(() => {
  return Object.keys(groupedResults.value).sort()
})

/** Select asset */
function selectAsset(asset: AssetSummary) {
  ws.activateAsset(asset)
}

function assetDisplayName(asset: AssetSummary): string {
  return AssetDisplayUtils.getAssetDisplayName(asset)
}

/** Keyboard navigation */
function onKeydown(e: KeyboardEvent) {
  const list = results.value
  if (!list.length) return

  if (e.key === 'ArrowDown') {
    e.preventDefault()
    highlightIndex.value = Math.min(highlightIndex.value + 1, list.length - 1)
    scrollToHighlight()
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    highlightIndex.value = Math.max(highlightIndex.value - 1, 0)
    scrollToHighlight()
  } else if (e.key === 'Enter' && highlightIndex.value >= 0) {
    e.preventDefault()
    selectAsset(list[highlightIndex.value])
  }
}

function scrollToHighlight() {
  nextTick(() => {
    const container = resultListRef.value
    if (!container) return
    // Results render grouped by class, so the DOM order differs from the flat
    // results array; select by the flat index via data-result-index instead of
    // the grouped DOM position.
    const target = container.querySelector<HTMLElement>(
      `.search-result-item[data-result-index="${highlightIndex.value}"]`,
    )
    if (target) target.scrollIntoView({ block: 'nearest' })
  })
}

/** Reset highlight on input */
watch(query, () => {
  highlightIndex.value = -1
})
</script>

<template>
  <div class="search-panel">
    <!-- Search bar -->
    <div class="search-bar">
      <div class="search-input-wrapper">
        <el-icon class="search-input-icon"><Search /></el-icon>
        <input
          ref="searchInputRef"
          v-model="query"
          type="text"
          class="search-input"
          :placeholder="t('home.searchAssetPlaceholder')"
          @keydown="onKeydown"
        />
        <button
          v-if="query"
          class="search-clear"
          @click="query = ''"
        >
          <el-icon><Close /></el-icon>
        </button>
      </div>
    </div>

    <!-- Stats -->
    <div v-if="query && results.length > 0" class="search-stats">
      {{ t('searchPanel.foundResults', { count: results.length }) }}
    </div>

    <!-- Empty state -->
    <div v-if="!ws.bundleMeta.value" class="panel-empty search-empty">
      {{ t('common.selectBundleFirst') }}
    </div>

    <div v-else-if="query && results.length === 0" class="panel-empty search-empty">
      <p>{{ t('searchPanel.noMatch', { query }) }}</p>
    </div>

    <div v-else-if="!query" class="panel-empty search-empty">
      <p class="search-hint">{{ t('searchPanel.hint') }}</p>
    </div>

    <!-- Search results -->
    <div
      v-if="query && results.length > 0"
      ref="resultListRef"
      class="search-results"
    >
      <div
        v-for="className in groupNames"
        :key="className"
        class="search-group"
      >
        <div class="search-group-title">
          <el-icon :size="14">
            <component :is="ClassIconUtils.getClassIcon(className)" />
          </el-icon>
          {{ className }}
          <span class="search-group-count">({{ groupedResults[className].length }})</span>
        </div>

        <div
          v-for="asset in groupedResults[className]"
          :key="asset.path_id"
          :data-result-index="results.indexOf(asset)"
          :class="[
            'search-result-item',
            { 'search-result-highlight': results.indexOf(asset) === highlightIndex },
          ]"
          @click="selectAsset(asset)"
          @contextmenu.prevent="ws.showContextMenu($event, asset)"
        >
          <div class="search-result-main">
            <span class="search-result-name">{{ assetDisplayName(asset) || t('common.unnamed') }}</span>
            <span class="search-result-path">{{ asset.path }}</span>
          </div>
          <span v-if="asset.class_name !== 'Texture2D' && asset.class_name !== 'Sprite' && asset.class_name !== 'SpriteMask'" class="search-result-size">{{ ws.getAssetSize(asset) }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.search-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
}

.search-bar {
  flex-shrink: 0;
  padding: 8px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}

.search-input-wrapper {
  display: flex;
  align-items: center;
  background: var(--el-fill-color-lighter);
  border: 1px solid var(--el-border-color-light);
  border-radius: 6px;
  padding: 0 8px;
  transition: border-color 0.2s, box-shadow 0.2s;
}

.search-input-wrapper:focus-within {
  border-color: var(--el-color-primary);
  box-shadow: 0 0 0 2px var(--el-color-primary-light-8);
  background: var(--el-bg-color);
}

.search-input-icon {
  color: var(--el-text-color-placeholder);
  font-size: 14px;
  flex-shrink: 0;
}

.search-input {
  flex: 1;
  border: none;
  outline: none;
  background: transparent;
  padding: 7px 6px;
  font-size: 13px;
  font-family: inherit;
  color: var(--el-text-color-primary);
  min-width: 0;
}

.search-input::placeholder {
  color: var(--el-text-color-placeholder);
}

.search-clear {
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  color: var(--el-text-color-placeholder);
  cursor: pointer;
  padding: 2px;
  border-radius: 4px;
  flex-shrink: 0;
}

.search-clear:hover {
  color: var(--el-text-color-primary);
  background: var(--el-fill-color);
}

.search-stats {
  flex-shrink: 0;
  padding: 5px 12px;
  font-size: 11px;
  color: var(--el-text-color-secondary);
  border-bottom: 1px solid var(--el-border-color-extra-light);
}

.search-empty {
  padding: 40px 16px !important;
}

.search-hint {
  color: var(--el-text-color-placeholder);
  font-size: 13px;
}

.search-results {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
}

.search-group {
  margin-bottom: 2px;
}

.search-group-title {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 5px 12px;
  font-size: 11px;
  font-weight: 600;
  color: var(--el-text-color-secondary);
  background: var(--el-fill-color-lighter);
  position: sticky;
  top: 0;
  z-index: 1;
  text-transform: uppercase;
  letter-spacing: 0.3px;
}

.search-group-count {
  font-weight: 400;
  color: var(--el-text-color-placeholder);
}

.search-result-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 5px 12px 5px 20px;
  cursor: pointer;
  font-size: 12px;
  transition: background 0.1s;
  border-bottom: 1px solid var(--el-border-color-extra-light);
}

.search-result-item:hover {
  background: var(--el-fill-color-light);
}

.search-result-highlight {
  background: var(--el-color-primary-light-9) !important;
}

.search-result-main {
  flex: 1;
  min-width: 0;
  overflow: hidden;
}

.search-result-name {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--el-text-color-primary);
  font-weight: 500;
  line-height: 1.4;
}

.search-result-path {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--el-text-color-placeholder);
  font-size: 10px;
  line-height: 1.3;
  margin-top: 1px;
}

.search-result-size {
  font-size: 11px;
  color: var(--el-text-color-secondary);
  margin-left: 8px;
  flex-shrink: 0;
}
</style>
