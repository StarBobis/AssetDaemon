<script setup lang="ts">
/**
 * ClassListPanel ->Asset type statistics panel (like AssetStudio Classes tab)
 *
 * Shows the name and count of all asset types in the current bundle.
 * Clicking a type filters the asset list to that type.
 * Clicking the same type again clears the filter.
 */

import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useWorkspace } from '../../composables/home/useWorkspace'
import { ClassIconUtils } from '../../utils/ClassIconUtils'

const ws = useWorkspace()
const { t } = useI18n()

/** Count assets by class name */
const typeStats = computed(() => {
  if (ws.isBundleAssetMapPaged.value) {
    return ws.bundleAssetClassStats.value
  }
  const meta = ws.bundleMeta.value
  if (!meta || !meta.assets) return []
  const counts: Record<string, number> = {}
  for (const asset of meta.assets) {
    counts[asset.class_name] = (counts[asset.class_name] || 0) + 1
  }
  return Object.entries(counts)
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => b.count - a.count) // Sort descending by count
})

/** Whether there is an active class filter */
const hasActiveFilter = computed(() => ws.activeAssetTypeFilters.value.length > 0)

/** Toggle type filter on click */
function toggleClassFilter(className: string) {
  if (ws.activeAssetTypeFilters.value.length === 1 && ws.activeAssetTypeFilters.value[0] === className) {
    ws.clearAssetTypeFilters()
  } else {
    ws.setAssetTypeFilter(className)
  }
}
</script>

<template>
  <div class="pane-header">
    <span>{{ t('home.assetTypes') }}</span>
    <span class="pane-header-right">
      <span class="panel-count" v-if="typeStats.length > 0">
        {{ typeStats.length }}
      </span>
    </span>
  </div>

  <div class="pane-scroll pane-classes">
    <!-- Empty state -->
    <div v-if="typeStats.length === 0" class="panel-empty">
      {{ t('common.selectBundleFirst') }}
    </div>

    <!-- Show clear button when filter is active -->
    <div v-if="hasActiveFilter" class="class-filter-bar">
      <span>{{ t('home.currentFilter', { name: ws.activeAssetTypeFilters.value.join(', ') }) }}</span>
      <el-button size="small" text @click="ws.clearAssetTypeFilters()">
        {{ t('common.clear') }}
      </el-button>
    </div>

    <!-- Type list -->
    <div
      v-for="stat in typeStats"
      :key="stat.name"
      :class="[
        'class-item',
        { 'class-item-active': ws.activeAssetTypeFilters.value.includes(stat.name) },
      ]"
      @click="toggleClassFilter(stat.name)"
    >
      <div class="class-item-left">
        <el-icon :size="16">
          <component :is="ClassIconUtils.getClassIcon(stat.name)" />
        </el-icon>
        <span class="class-item-name">{{ stat.name }}</span>
      </div>
      <span class="class-item-count">{{ stat.count }}</span>
    </div>
  </div>
</template>

<style scoped>
.pane-classes {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
}

.class-filter-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 12px;
  font-size: 12px;
  color: var(--el-color-primary);
  background: var(--el-color-primary-light-9);
  border-bottom: 1px solid var(--el-color-primary-light-7);
}

.class-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 5px 12px;
  font-size: 12px;
  cursor: pointer;
  transition: background 0.12s;
  border-bottom: 1px solid var(--el-border-color-extra-light);
}

.class-item:hover {
  background: var(--el-fill-color-light);
}

.class-item-active {
  background: var(--el-color-primary-light-9);
  color: var(--el-color-primary);
  font-weight: 600;
}

.class-item-left {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  min-width: 0;
}

.class-item-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.class-item-count {
  font-size: 11px;
  color: var(--el-text-color-secondary);
  background: var(--el-fill-color-lighter);
  padding: 0 6px;
  border-radius: 8px;
  line-height: 18px;
  flex-shrink: 0;
}
</style>
