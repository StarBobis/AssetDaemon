<script setup lang="ts">
/**
 * Shared context menu for asset rows in both the current Bundle list and All Assets.
 */

import { onMounted, onUnmounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { useWorkspace } from '../../composables/home/useWorkspace'
import { useExport } from '../../composables/home/useExport'
import { AssetDisplayUtils } from '../../utils/AssetDisplayUtils'
import type { AssetSummary, ExportAssetRef } from '../../types'

const props = defineProps<{
  /** Assets represented by the owning panel, used by Export All. */
  assets: AssetSummary[]
}>()

const ws = useWorkspace()
const exp = useExport()
const { t } = useI18n()

function onDocumentClick() {
  ws.hideContextMenu()
}

onMounted(() => document.addEventListener('click', onDocumentClick))
onUnmounted(() => document.removeEventListener('click', onDocumentClick))

function getAssetBundlePath(asset: AssetSummary): string {
  return asset.source_bundle_path || ws.selectedBundlePath.value
}

function toExportRef(asset: AssetSummary): ExportAssetRef | null {
  const bundlePath = getAssetBundlePath(asset)
  if (!bundlePath) return null
  return {
    bundle_path: bundlePath,
    path_id: asset.path_id,
    class_name: asset.class_name,
    asset_name: AssetDisplayUtils.getAssetExportName(asset),
  }
}

function handleExportThis() {
  const clicked = ws.contextMenuAsset.value
  const exportRef = clicked ? toExportRef(clicked) : null
  if (exportRef) {
    exp.openExportDialog([exportRef])
  }
  ws.hideContextMenu()
}

function handleExportSelected() {
  if (exp.selectedAssets.value.length > 0) {
    exp.openExportDialog([...exp.selectedAssets.value])
    ws.hideContextMenu()
    return
  }

  const clicked = ws.contextMenuAsset.value
  const exportRef = clicked ? toExportRef(clicked) : null
  if (exportRef) {
    exp.openExportDialog([exportRef])
  }
  ws.hideContextMenu()
}

function handleExportAll() {
  const assets = props.assets
    .map(toExportRef)
    .filter((asset): asset is ExportAssetRef => asset !== null)
  exp.openExportDialog(assets)
  ws.hideContextMenu()
}

</script>

<template>
  <Teleport to="body">
    <div
      v-if="ws.contextMenuVisible.value"
      class="asset-context-menu"
      :style="{ left: ws.contextMenuX.value + 'px', top: ws.contextMenuY.value + 'px' }"
      @click.stop
    >
      <div class="context-menu-item" @click="ws.openPropsDialog()">
        {{ t('contextMenu.viewProperties') }}
      </div>
      <div class="context-menu-sep"></div>
      <div class="context-menu-item" @click="handleExportThis()">
        {{ t('contextMenu.exportThis') }}
      </div>
      <div class="context-menu-item" @click="handleExportSelected()">
        {{ t('contextMenu.exportSelected', { count: exp.selectedAssets.value.length }) }}
      </div>
      <div class="context-menu-item" @click="handleExportAll()">
        {{ t('contextMenu.exportAll') }}
      </div>
      <div class="context-menu-sep"></div>
      <div class="context-menu-item" @click="ws.clearSingleMeshCache()">
        {{ t('contextMenu.clearThisMeshCache') }}
      </div>
      <div class="context-menu-item" @click="ws.clearAllMeshCache()">
        {{ t('contextMenu.clearAllMeshCache') }}
      </div>
    </div>
  </Teleport>

</template>
