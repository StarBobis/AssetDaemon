<script setup lang="ts">
/**
 * WorkSpace ->Unity AssetBundle Resource Browser
 *
 * This is the entry component for the page, responsible for:
 *   1. Importing all composables (shared singleton state)
 *   2. Orchestrating the overall layout (toolbar + 3-Tab left panel + preview)
 *   3. Lifecycle management (initialize panel/window sizes, load data, listen for resize)
 *
 * Sub-components:
 *   - FileListPanel  Top-left: File list
 *   - AssetListPanel  Bottom-left: Asset list
 *   - PreviewPanel    Top-right: Preview area
 *   - PropertiesPanel Bottom-right: Properties panel
 *
 * Composables:
 *   - useWorkspace    Core business logic
 *   - usePanelLayout  Draggable splitters + panel/window size persistence
 *   - useLogSystem    Logging system
 */

import { computed, onMounted, onUnmounted, watch, ref } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { load } from '@tauri-apps/plugin-store'
import { ElMessageBox } from 'element-plus'
import {
  FolderOpened,
  Refresh,
  Delete,
  DocumentAdd,
  MapLocation,
} from '@element-plus/icons-vue'
import { invoke, Channel } from '@tauri-apps/api/core'
import { useI18n } from 'vue-i18n'
import FileListPanel from './components/home/FileListPanel.vue'
import SceneHierarchyPanel from './components/home/SceneHierarchyPanel.vue'
import AllAssetsPanel from './components/home/AllAssetsPanel.vue'
import AssetListPanel from './components/home/AssetListPanel.vue'
import PreviewPanel from './components/home/PreviewPanel.vue'
import DumpPanel from './components/home/DumpPanel.vue'
import ExportDialog from './components/home/ExportDialog.vue'
import { useWorkspace } from './composables/home/useWorkspace'
import { usePanelLayout } from './composables/home/usePanelLayout'
import { useLogSystem } from './composables/useLogSystem'
import { AssetMapCacheUtils } from './utils/AssetMapCacheUtils'

// ============================================================
// Initialize all composables (shared singleton state)
// ============================================================

const ws = useWorkspace()
const layout = usePanelLayout()
const { addLog, stopSignal } = useLogSystem()
const { t } = useI18n()
const SKIP_RESS_LIST_BUILD_KEY = 'build-map-skip-ress-list-build'

/// Whether currently building AssetMap
const buildingMap = ref(false)
const loadingMapSummary = ref(false)
const skipResSListBuild = ref(false)
const mapSettingsVisible = ref(false)

type MapSummary = {
  built_at: number
  built_at_formatted: string
  bundle_count: number
  asset_count: number
  parsed_count: number
  cancelled?: boolean
}

const mapSummary = ref<MapSummary | null>(null)

const mapReady = computed(() => !!mapSummary.value)
const mapButtonText = computed(() => {
  if (buildingMap.value) return t('home.mapBuilding')
  if (loadingMapSummary.value) return t('home.mapChecking')
  if (!mapSummary.value) return t('home.mapMissing')
  return t('home.mapAssetCount', { count: mapSummary.value.asset_count.toLocaleString() })
})

const mapStatusText = computed(() => {
  if (buildingMap.value) return t('home.mapStatusBuilding')
  if (loadingMapSummary.value) return t('home.mapStatusChecking')
  if (!mapSummary.value) return t('home.mapStatusMissing')
  return t('home.mapStatusReady', {
    bundles: mapSummary.value.bundle_count.toLocaleString(),
    assets: mapSummary.value.asset_count.toLocaleString(),
  })
})

const mapBuiltAtText = computed(() => {
  if (!mapSummary.value) return t('home.mapBuildHint')
  return t('home.mapBuiltAt', { time: mapSummary.value.built_at_formatted })
})

async function refreshMapSummary(options?: { silent?: boolean }) {
  if (!ws.workDir.value) {
    mapSummary.value = null
    return
  }

  loadingMapSummary.value = true
  try {
    const assetMapCacheRoot = await AssetMapCacheUtils.getCacheRoot()
    mapSummary.value = await invoke<MapSummary | null>('get_map_summary', {
      workspaceDir: ws.workDir.value,
      assetMapCacheRoot,
    })
  } catch (e) {
    mapSummary.value = null
    if (!options?.silent) {
      addLog(t('home.mapSummaryFailed', { error: e }), 'warn')
    }
  } finally {
    loadingMapSummary.value = false
  }
}

/// Build lightweight search index (AssetMap)
async function handleBuildMap() {
  if (!ws.workDir.value) { addLog(t('home.selectWorkspaceFirst'), 'warn'); return }
  mapSettingsVisible.value = false
  buildingMap.value = true
  addLog(t('home.mapBuildStarting'), 'info')
  const start = Date.now()
  let lastMapProgressLog = 0
  try {
    const ch = new Channel<{ step: string; message: string }>()
    ch.onmessage = (p) => {
      const now = Date.now()
      if (p.step !== 'write' || now - lastMapProgressLog >= 5000) {
        const remainingLabel = estimateBuildMapRemainingTime(p.message, start, now)
        const suffix = remainingLabel ? t('home.roughEtaSuffix', { time: remainingLabel }) : ''
        addLog(`[Map] ${p.message}${suffix}`, 'info')
        lastMapProgressLog = now
      }
    }
    const assetMapCacheRoot = await AssetMapCacheUtils.getCacheRoot()
    const summary = await invoke<MapSummary>('build_asset_map', {
      workspaceDir: ws.workDir.value,
      progress: ch,
      memoryLimitBytes: 0,
      assetMapCacheRoot,
      skipRessListBuild: skipResSListBuild.value,
    })
    const elapsed = ((Date.now() - start) / 1000).toFixed(1)
    mapSummary.value = summary
    ws.notifyAssetMapChanged()
    if (summary.cancelled) {
      addLog(t('home.mapBuildStopped', { parsed: summary.parsed_count, assets: summary.asset_count, seconds: elapsed }), 'warn')
    } else {
      addLog(t('home.mapBuildComplete', { bundles: summary.bundle_count, assets: summary.asset_count, seconds: elapsed }), 'success')
      addLog(t('home.mapBuildBenefit'), 'info')
    }
  } catch (e) {
    addLog(t('home.mapBuildFailed', { error: e }), 'error')
  } finally {
    buildingMap.value = false
    await refreshMapSummary({ silent: true })
  }
}

async function loadSkipResSListBuild(): Promise<void> {
  try {
    const store = await load('settings.json', { defaults: {}, autoSave: true })
    skipResSListBuild.value = await store.get(SKIP_RESS_LIST_BUILD_KEY) === true
  } catch (_) {
    skipResSListBuild.value = false
  }
}

async function setSkipResSListBuild(value: string | number | boolean): Promise<void> {
  skipResSListBuild.value = value === true
  try {
    const store = await load('settings.json', { defaults: {}, autoSave: true })
    await store.set(SKIP_RESS_LIST_BUILD_KEY, skipResSListBuild.value)
    await store.save()
  } catch (e) {
    console.error('Failed to save Build Index ResSList option:', e)
  }
}

function estimateBuildMapRemainingTime(message: string, startedAtMs: number, nowMs: number): string {
  const progressMatch = message.match(/started\s+(\d+)\/(\d+)\s+bundles/i)
  if (!progressMatch) return ''

  const current = Number(progressMatch[1])
  const total = Number(progressMatch[2])
  const elapsedSeconds = Math.max(1, (nowMs - startedAtMs) / 1000)
  if (!Number.isFinite(current) || !Number.isFinite(total) || current <= 0 || total <= current) {
    return ''
  }

  const bundlesPerSecond = current / elapsedSeconds
  if (bundlesPerSecond <= 0) return ''

  const remainingSeconds = Math.ceil((total - current) / bundlesPerSecond)
  return formatBuildMapDuration(remainingSeconds)
}

function formatBuildMapDuration(totalSeconds: number): string {
  const seconds = Math.max(0, Math.floor(totalSeconds))
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  const restSeconds = seconds % 60
  if (hours > 0) {
    return `${hours}h ${String(minutes).padStart(2, '0')}m ${String(restSeconds).padStart(2, '0')}s`
  }
  if (minutes > 0) {
    return `${minutes}m ${String(restSeconds).padStart(2, '0')}s`
  }
  return `${restSeconds}s`
}

watch(stopSignal, () => {
  if (!buildingMap.value) return
  buildingMap.value = false
  loadingMapSummary.value = false
  mapSettingsVisible.value = false
  addLog(t('home.mapBuildStopRequested'), 'warn')
})

async function handleClearMap() {
  if (!ws.workDir.value) { addLog(t('home.selectWorkspaceFirst'), 'warn'); return }
  try {
    await ElMessageBox.confirm(
      t('home.clearMapConfirmMessage'),
      t('home.clearMapConfirmTitle'),
      {
        confirmButtonText: t('home.clearMapConfirmButton'),
        cancelButtonText: t('common.cancel'),
        type: 'warning',
        confirmButtonClass: 'el-button--danger',
      },
    )
    addLog(t('home.mapClearing'), 'info')
    const assetMapCacheRoot = await AssetMapCacheUtils.getCacheRoot()
    await invoke('clear_asset_map', { workspaceDir: ws.workDir.value, assetMapCacheRoot })
    addLog(t('home.mapCleared'), 'success')
    mapSettingsVisible.value = false
    mapSummary.value = null
    ws.notifyAssetMapChanged()
  } catch (e) {
    if (e === 'cancel' || e === 'close') return
    addLog(t('home.mapClearFailed', { error: e }), 'error')
  } finally {
    await refreshMapSummary({ silent: true })
  }
}

// ============================================================
// Lifecycle
// ============================================================

onMounted(async () => {
  // Step 1: Load persisted panel layout (panel widths, splitter positions, etc.)
  await layout.loadPanelLayout()

  // Step 2: Load type scan configuration (persistent cache + concurrency)
  await ws.loadTypeScanConfig()

  // Step 3: Load bundle metadata cache limit configuration
  await ws.loadBundleCacheConfig()

  // Step 4: Load Build Index options
  await loadSkipResSListBuild()

  // Step 5: Restore workspace history and last workspace directory
  const s = await ws.getStore()
  await ws.loadWorkspaceHistory()
  const savedDir = await s.get('workspace-dir')
  if (typeof savedDir === 'string' && savedDir) {
    await ws.restoreWorkspaceDir(savedDir)
    await refreshMapSummary({ silent: true })
    const restoringDir = savedDir
    void ws.loadBundleFiles().then(async () => {
      if (ws.workDir.value !== restoringDir) return
      // After file list loads, restore the last used type filters.
      // Must be called after loadBundleFiles because loadBundleFiles clears activeFilters.
      const savedFilters = await s.get('active-filters')
      const legacyFilter = await s.get('active-filter')
      if (Array.isArray(savedFilters)) {
        ws.setActiveFilters(savedFilters.filter((name): name is string => typeof name === 'string'), { allowScanFallback: false })
      } else if (typeof legacyFilter === 'string' && legacyFilter) {
        ws.setActiveFilters([legacyFilter], { allowScanFallback: false })
      }
    })
  }

  // Step 6: Bind keyboard events (file list up/down navigation)
  document.addEventListener('keydown', ws.onKeydown)

  // Step 7: Listen for file drag-drop
  try {
    const appWindow = getCurrentWindow()
    appWindow.onDragDropEvent(async (event) => {
      if (event.payload.type === 'drop') {
        const paths: string[] = event.payload.paths
        if (paths && paths.length > 0) {
          await ws.addBundlePaths(paths)
        }
      }
    })
  } catch (e) {
    console.error('Window resize or drag-drop listener failed:', e)
  }
})

onUnmounted(() => {
  document.removeEventListener('keydown', ws.onKeydown)
})

// ============================================================
// Dump Tab: Trigger loading when switching to Dump (asset-change watch already handles cache)
// ============================================================

watch(() => ws.rightTab.value, (newTab) => {
  if (newTab === 'dump' && ws.dumpTargetAsset.value) {
    ws.fetchDumpData()
  }
})

watch(() => ws.workDir.value, async (newDir, oldDir) => {
  if (newDir === oldDir) return
  await refreshMapSummary({ silent: true })
})
</script>

<template>
  <div class="workspace-page">
    <!-- ============================================================
         Top toolbar
    ============================================================ -->
    <div class="toolbar">
      <div class="toolbar-workspace">
        <el-select
          class="toolbar-path-input"
          :model-value="ws.workDir.value"
          :placeholder="t('home.selectDirectory')"
          :title="ws.workDir.value || t('home.selectDirectory')"
          filterable
          :disabled="ws.workspaceHistory.value.length === 0"
          @change="(value: string) => ws.selectWorkspaceFromHistory(value)"
        >
          <el-option
            v-for="dir in ws.workspaceHistory.value"
            :key="dir"
            :label="dir"
            :value="dir"
          />
        </el-select>
        <el-button
          class="toolbar-icon-btn"
          :title="t('home.selectDirectory')"
          :icon="FolderOpened"
          @click="ws.selectWorkDir"
        />
        <el-button
          class="toolbar-icon-btn"
          :title="t('home.selectFiles')"
          :icon="DocumentAdd"
          @click="ws.selectBundleFiles"
        />
        <el-button
          class="toolbar-icon-btn"
          :disabled="!ws.workDir.value && ws.bundleFiles.value.length === 0"
          :loading="ws.isLoading.value"
          :title="t('home.refresh')"
          :icon="Refresh"
          @click="ws.handleRefresh"
        />
        <el-button
          class="toolbar-icon-btn"
          :disabled="!ws.workDir.value && ws.bundleFiles.value.length === 0"
          type="danger"
          :title="t('common.clear')"
          :icon="Delete"
          @click="ws.removeCurrentWorkspaceFromHistory"
        />
        <el-popover
          v-model:visible="mapSettingsVisible"
          trigger="click"
          placement="bottom-start"
          :width="300"
          popper-class="map-settings-popover"
        >
          <template #reference>
            <el-button
              class="toolbar-build-btn"
              :disabled="!ws.workDir.value || ws.bundleFiles.value.length === 0"
              :loading="buildingMap || loadingMapSummary"
              type="success"
              :plain="!mapReady"
            >
              <el-icon class="btn-icon"><MapLocation /></el-icon>
              {{ mapButtonText }}
            </el-button>
          </template>
          <div class="map-settings-card">
            <div class="map-settings-header">
              <span>{{ t('home.mapSettings') }}</span>
              <el-tag size="small" :type="mapReady ? 'success' : 'warning'" effect="plain">
                {{ mapReady ? t('home.mapReady') : t('home.mapMissing') }}
              </el-tag>
            </div>
            <div class="map-settings-meta">
              <div class="map-settings-row">
                <span>{{ t('home.mapStatus') }}</span>
                <strong>{{ mapStatusText }}</strong>
              </div>
              <div class="map-settings-row">
                <span>{{ t('home.mapBuildTime') }}</span>
                <strong>{{ mapBuiltAtText }}</strong>
              </div>
            </div>
            <el-checkbox
              class="map-settings-check"
              :model-value="skipResSListBuild"
              @update:model-value="setSkipResSListBuild"
            >
              {{ t('home.skipResSListBuild') }}
            </el-checkbox>
            <div class="map-settings-actions">
              <el-button
                size="small"
                type="success"
                :loading="buildingMap"
                :disabled="!ws.workDir.value || ws.bundleFiles.value.length === 0"
                @click="handleBuildMap"
              >
                {{ mapReady ? t('home.rebuildMap') : t('home.buildMap') }}
              </el-button>
              <el-button
                size="small"
                type="danger"
                plain
                @click="handleClearMap"
              >
                {{ t('home.clearMap') }}
              </el-button>
            </div>
          </div>
        </el-popover>
      </div>
      <div class="toolbar-reserved"></div>
    </div>

    <!-- ============================================================
         Dual-pane main layout (left 5-Tab panel -> right preview + properties)
         Draggable splitters: left panel width, right panel top/bottom ratio
    ============================================================ -->
    <div class="main-layout" ref="layout.mainLayoutRef">
      <!-- Left panel: 2 Tabs (files/assets + hierarchy), default 680px -->
      <div class="panel panel-left" :style="{ width: layout.leftPanelWidth.value + 'px' }">
        <!-- Tab switch bar -->
        <div class="left-tab-bar">
          <button
            class="left-tab"
            :class="{ active: ws.activeTab.value === 'files' }"
            @click="ws.activeTab.value = 'files'"
            :title="t('home.filesTabTitle')"
          >
            <svg class="tab-icon" viewBox="0 0 16 16" width="14" height="14">
              <path fill="currentColor" d="M2 1.75C2 .784 2.784 0 3.75 0h5.586c.464 0 .909.184 1.237.513l2.914 2.914c.329.328.513.773.513 1.237v9.586A1.75 1.75 0 0 1 12.25 16h-8.5A1.75 1.75 0 0 1 2 14.25Zm1.75-.25a.25.25 0 0 0-.25.25v12.5c0 .138.112.25.25.25h8.5a.25.25 0 0 0 .25-.25V6h-2.75A1.75 1.75 0 0 1 8 4.25V1.5Z"/>
            </svg>
            <span class="tab-label">{{ t('home.filesTab') }}</span>
          </button>
          <button
            class="left-tab"
            :class="{ active: ws.activeTab.value === 'hierarchy' }"
            @click="ws.activeTab.value = 'hierarchy'"
            :title="t('home.hierarchyTabTitle')"
          >
            <svg class="tab-icon" viewBox="0 0 16 16" width="14" height="14">
              <path fill="currentColor" d="M1.5 1.75C1.5.784 2.284 0 3.25 0h1.5c.966 0 1.75.784 1.75 1.75v1.5c0 .066-.003.13-.01.194l3.38 1.936A1.75 1.75 0 0 1 11.75 4h1.5c.966 0 1.75.784 1.75 1.75v1.5c0 .966-.784 1.75-1.75 1.75h-1.5a1.75 1.75 0 0 1-1.729-1.506L7.07 5.322a1.75 1.75 0 0 1-.07.028v4.65a1.75 1.75 0 0 1-.88 1.522l-2.62 1.443c.047.157.08.324.08.5v1.5c0 .966-.784 1.75-1.75 1.75h-1.5A1.75 1.75 0 0 1 0 14.75v-1.5C0 12.284.784 11.5 1.75 11.5h1.5c.065 0 .128.004.19.01l2.62-1.443A1.75 1.75 0 0 1 6 10.048V6.18c-.302-.115-.56-.32-.757-.58L1.69 4.444A1.75 1.75 0 0 1 1.5 4.5v-1.5c0-.466.127-.902.345-1.275l1.377.803A1.75 1.75 0 0 1 3.25 4h-1.5A1.75 1.75 0 0 1 0 2.25v-1.5Z"/>
            </svg>
            <span class="tab-label">{{ t('home.hierarchyTab') }}</span>
          </button>
          <button
            class="left-tab"
            :class="{ active: ws.activeTab.value === 'allassets' }"
            @click="ws.activeTab.value = 'allassets'"
            :title="t('home.allAssetsTabTitle')"
          >
            <svg class="tab-icon" viewBox="0 0 16 16" width="14" height="14">
              <path fill="currentColor" d="M2 3.5A1.5 1.5 0 0 1 3.5 2h2A1.5 1.5 0 0 1 7 3.5v2A1.5 1.5 0 0 1 5.5 7h-2A1.5 1.5 0 0 1 2 5.5v-2Zm2.5-.5a.5.5 0 0 0-.5.5v2a.5.5 0 0 0 .5.5h2a.5.5 0 0 0 .5-.5v-2a.5.5 0 0 0-.5-.5h-2Zm4.5.5A1.5 1.5 0 0 1 10.5 2h2A1.5 1.5 0 0 1 14 3.5v2A1.5 1.5 0 0 1 12.5 7h-2A1.5 1.5 0 0 1 9 5.5v-2Zm2.5-.5a.5.5 0 0 0-.5.5v2a.5.5 0 0 0 .5.5h2a.5.5 0 0 0 .5-.5v-2a.5.5 0 0 0-.5-.5h-2ZM2 10.5A1.5 1.5 0 0 1 3.5 9h2A1.5 1.5 0 0 1 7 10.5v2A1.5 1.5 0 0 1 5.5 14h-2A1.5 1.5 0 0 1 2 12.5v-2Zm2.5-.5a.5.5 0 0 0-.5.5v2a.5.5 0 0 0 .5.5h2a.5.5 0 0 0 .5-.5v-2a.5.5 0 0 0-.5-.5h-2Zm4.5.5A1.5 1.5 0 0 1 10.5 9h2a1.5 1.5 0 0 1 1.5 1.5v2a1.5 1.5 0 0 1-1.5 1.5h-2A1.5 1.5 0 0 1 9 12.5v-2Zm2.5-.5a.5.5 0 0 0-.5.5v2a.5.5 0 0 0 .5.5h2a.5.5 0 0 0 .5-.5v-2a.5.5 0 0 0-.5-.5h-2Z"/>
            </svg>
            <span class="tab-label">{{ t('home.allAssetsTab') }}</span>
          </button>
        </div>

        <!-- Tab content area: fill remaining space -->
        <div class="left-tab-body">
          <!-- Files tab: left-right layout ->File list | Asset list -->
          <div v-if="ws.activeTab.value === 'files'" class="files-assets-split">
            <div class="fas-left" :style="{ width: layout.fileListWidth.value + 'px' }">
              <FileListPanel />
            </div>
            <div
              class="fas-splitter-v"
              @mousedown.prevent="layout.startDragFilesAssets"
            ></div>
            <div class="fas-right">
              <AssetListPanel />
            </div>
          </div>
          <KeepAlive>
            <SceneHierarchyPanel v-if="ws.activeTab.value === 'hierarchy'" />
            <AllAssetsPanel v-else-if="ws.activeTab.value === 'allassets'" />
          </KeepAlive>
        </div>
      </div>

      <!-- ====== Draggable vertical splitter (left panel / right panel) ====== -->
      <div
        class="splitter-v"
        @mousedown.prevent="layout.startDragLeftRight"
      >
        <div class="splitter-v-handle"></div>
      </div>

      <!-- Right panel: Preview / Dump dual Tab -->
      <div class="panel panel-right">
        <!-- Right panel Tab switch bar -->
        <div class="right-tab-bar">
          <button
            class="right-tab"
            :class="{ active: ws.rightTab.value === 'preview' }"
            @click="ws.rightTab.value = 'preview'"
            :title="t('home.previewTabTitle')"
          >
            <svg class="tab-icon" viewBox="0 0 16 16" width="14" height="14">
              <path fill="currentColor" d="M8 3.5a.5.5 0 0 0-1 0V5H5.5a.5.5 0 0 0 0 1H7v1.5a.5.5 0 0 0 1 0V6h1.5a.5.5 0 0 0 0-1H8V3.5z"/>
              <path fill="currentColor" d="M13.5 3a.5.5 0 0 1 .5.5v9a.5.5 0 0 1-.5.5h-11a.5.5 0 0 1-.5-.5v-9a.5.5 0 0 1 .5-.5h11zm-11-1A1.5 1.5 0 0 0 1 3.5v9A1.5 1.5 0 0 0 2.5 14h11a1.5 1.5 0 0 0 1.5-1.5v-9A1.5 1.5 0 0 0 13.5 2h-11z"/>
            </svg>
            <span class="tab-label">{{ t('home.previewTab') }}</span>
          </button>
          <button
            class="right-tab"
            :class="{ active: ws.rightTab.value === 'dump' }"
            @click="ws.rightTab.value = 'dump'"
            :title="t('home.dumpTabTitle')"
          >
            <svg class="tab-icon" viewBox="0 0 16 16" width="14" height="14">
              <path fill="currentColor" d="M2 1.75C2 .784 2.784 0 3.75 0h8.5C13.216 0 14 .784 14 1.75v12.5A1.75 1.75 0 0 1 12.25 16h-8.5A1.75 1.75 0 0 1 2 14.25V1.75zm1.75-.25a.25.25 0 0 0-.25.25v12.5c0 .138.112.25.25.25h8.5a.25.25 0 0 0 .25-.25V1.75a.25.25 0 0 0-.25-.25h-8.5zM4 3.5a.5.5 0 0 1 .5-.5h7a.5.5 0 0 1 0 1h-7a.5.5 0 0 1-.5-.5zm0 3a.5.5 0 0 1 .5-.5h7a.5.5 0 0 1 0 1h-7a.5.5 0 0 1-.5-.5zm0 3a.5.5 0 0 1 .5-.5h4a.5.5 0 0 1 0 1h-4a.5.5 0 0 1-.5-.5z"/>
            </svg>
            <span class="tab-label">{{ t('home.dumpTab') }}</span>
          </button>
        </div>

        <!-- Right panel Tab content -->
        <div class="right-tab-body">
          <PreviewPanel v-if="ws.rightTab.value === 'preview'" />
          <DumpPanel v-else-if="ws.rightTab.value === 'dump'" />
        </div>
      </div>
    </div>

  </div>

  <!-- Properties dialog -->
  <el-dialog
    v-model="ws.showPropsDialog.value"
    :title="t('home.properties')"
    width="420px"
    :close-on-click-modal="true"
    align-center
    @closed="ws.dialogAsset.value = null"
  >
    <div v-if="ws.dialogAsset.value" class="props-dialog-body">
      <div class="props-dialog-row">
        <span class="props-dialog-key">{{ t('home.name') }}</span>
        <span class="props-dialog-val">{{ ws.dialogAsset.value.name || t('common.unnamed') }}</span>
      </div>
      <div class="props-dialog-row">
        <span class="props-dialog-key">{{ t('home.path') }}</span>
        <span class="props-dialog-val props-path">{{ ws.dialogAsset.value.path }}</span>
      </div>
      <div class="props-dialog-row">
        <span class="props-dialog-key">{{ t('home.type') }}</span>
        <span class="props-dialog-val">{{ ws.dialogAsset.value.class_name }}</span>
      </div>
      <div class="props-dialog-row">
        <span class="props-dialog-key">{{ t('home.classId') }}</span>
        <span class="props-dialog-val">{{ ws.dialogAsset.value.class_id }}</span>
      </div>
      <div class="props-dialog-row">
        <span class="props-dialog-key">{{ t('home.pathId') }}</span>
        <span class="props-dialog-val props-mono">{{ ws.dialogAsset.value.path_id }}</span>
      </div>
      <div class="props-dialog-row">
        <span class="props-dialog-key">{{ t('home.dataSize') }}</span>
        <span class="props-dialog-val">{{ ws.getAssetSize(ws.dialogAsset.value) }}</span>
      </div>
    </div>
    <div v-else class="props-dialog-body">
      <span class="panel-empty">{{ t('home.noAssetInfo') }}</span>
    </div>
  </el-dialog>

  <!-- Batch export dialog -->
  <ExportDialog />
</template>

<style src="./styles/Home.css"></style>
