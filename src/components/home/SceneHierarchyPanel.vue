<script setup lang="ts">
/**
 * SceneHierarchyPanel ->Scene hierarchy tree panel
 *
 * Calls Rust backend to parse GameObject/Transform tree, renders with el-tree.
 * Click a node to select the corresponding asset.
 */

import { watch, ref, onMounted } from 'vue'
import { Refresh } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import { useWorkspace } from '../../composables/home/useWorkspace'
import type { SceneNode } from '../../types'

const ws = useWorkspace()
const { t } = useI18n()

/** el-tree node-key */
const TREE_KEY = 'path_id'

/** Expanded node key set (all expanded by default) */
const expandedKeys = ref<string[]>([])

/** Whether hierarchy has been loaded for current path (avoid reloading) */
const hierarchyLoadedForPath = ref('')

async function loadHierarchy(bundlePath: string) {
  if (!bundlePath) return
  hierarchyLoadedForPath.value = bundlePath
  await ws.loadSceneHierarchy(bundlePath)
  if (ws.sceneHierarchy.value.length > 0) {
    expandedKeys.value = collectAllKeys(ws.sceneHierarchy.value)
  }
}

onMounted(() => {
  if (ws.selectedBundlePath.value && hierarchyLoadedForPath.value !== ws.selectedBundlePath.value) {
    loadHierarchy(ws.selectedBundlePath.value)
  }
})

watch(() => ws.selectedBundlePath.value, (newPath) => {
  if (!newPath) { expandedKeys.value = []; return }
  loadHierarchy(newPath)
})

watch(() => ws.activeTab.value, (tab) => {
  if (tab === 'hierarchy' && ws.selectedBundlePath.value && hierarchyLoadedForPath.value !== ws.selectedBundlePath.value) {
    loadHierarchy(ws.selectedBundlePath.value)
  }
})

/** Collect all path_ids to expand all */
function collectAllKeys(nodes: SceneNode[]): string[] {
  const keys: string[] = []
  for (const n of nodes) {
    keys.push(String(n.path_id))
    keys.push(...collectAllKeys(n.children))
  }
  return keys
}

/** Recursively render tree node */
function renderNode(h: any, { data }: { data: SceneNode }) {
  return h('div', { class: 'hierarchy-node' }, [
    h('span', { class: 'hierarchy-node-icon' }, t('sceneHierarchy.nodeIcon')),
    h('span', { class: 'hierarchy-node-name' }, data.name || t('common.unnamed')),
    data.components.length > 0
      ? h('span', { class: 'hierarchy-node-comps' },
          data.components.map(c => c.class_name).join(', '))
      : null,
  ])
}

/** Node click handler */
function onNodeClick(nodeData: SceneNode) {
  // Try to find asset with matching path_id in the current bundle
  const meta = ws.bundleMeta.value
  if (!meta) return
  const asset = meta.assets.find(a => a.path_id === String(nodeData.path_id))
  if (asset) {
    void ws.activateAsset(asset)
  } else {
    // Find the first associated component
    const compAsset = meta.assets.find(a => nodeData.components.some(c => c.path_id === a.path_id))
    if (compAsset) void ws.activateAsset(compAsset)
  }
}
</script>

<template>
  <div class="pane-header">
    <span>{{ t('sceneHierarchy.title') }}</span>
    <span class="pane-header-right">
      <el-button
        v-if="ws.selectedBundlePath.value"
        size="small"
        circle
        :loading="ws.hierarchyLoading.value"
        @click="ws.loadSceneHierarchy(ws.selectedBundlePath.value)"
      >
        <el-icon><Refresh /></el-icon>
      </el-button>
    </span>
  </div>

  <div class="pane-scroll pane-hierarchy">
    <!-- No bundle selected -->
    <div v-if="!ws.selectedBundlePath.value" class="panel-empty">
      {{ t('common.selectBundleFirst') }}
    </div>

    <!-- Loading -->
    <div v-else-if="ws.hierarchyLoading.value" class="panel-loading">
      <el-icon class="is-loading" :size="24"><Refresh /></el-icon>
      <p>{{ t('sceneHierarchy.parsing') }}</p>
    </div>

    <!-- No GameObject -->
    <div v-else-if="ws.sceneHierarchy.value.length === 0" class="panel-empty">
      <p>{{ t('sceneHierarchy.noData') }}</p>
      <p class="hierarchy-hint">{{ t('sceneHierarchy.noObjects') }}</p>
    </div>

    <!-- Hierarchy tree -->
    <el-tree
      v-else
      :data="ws.sceneHierarchy.value"
      :props="{ children: 'children', label: 'name' }"
      :node-key="TREE_KEY"
      :default-expanded-keys="expandedKeys"
      :render-content="renderNode"
      highlight-current
      @node-click="onNodeClick"
      class="hierarchy-tree"
    />
  </div>
</template>

<style scoped>
.pane-hierarchy {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 0;
}

.hierarchy-hint {
  font-size: 11px;
  color: var(--el-text-color-placeholder);
  margin-top: 4px;
}

.hierarchy-tree {
  font-size: 12px;
}

.hierarchy-node {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 2px 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.hierarchy-node-icon {
  flex-shrink: 0;
  font-size: 12px;
}

.hierarchy-node-name {
  font-weight: 500;
  color: var(--el-text-color-primary);
}

.hierarchy-node-comps {
  font-size: 10px;
  color: var(--el-text-color-placeholder);
  margin-left: 6px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
