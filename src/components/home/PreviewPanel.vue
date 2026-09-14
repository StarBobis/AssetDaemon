<script setup lang="ts">
/**
 * PreviewPanel ->Right preview area
 *
 * Shows preview based on selected asset type:
 *   - Mesh / GameObject ->3D preview (Model3D component)
 *   - Texture2D / Sprite ->Texture image preview
 *   - TextAsset / MonoBehaviour ->Text preview
 *   - Other types ->Unsupported message
 */

import { computed, defineAsyncComponent, ref, watch } from 'vue'
import { VideoCamera, PictureFilled, Document, Refresh } from '@element-plus/icons-vue'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { copyFile, mkdir } from '@tauri-apps/plugin-fs'
import { revealItemInDir, openPath } from '@tauri-apps/plugin-opener'
import { basename, join } from '@tauri-apps/api/path'
import { useI18n } from 'vue-i18n'
import { useWorkspace } from '../../composables/home/useWorkspace'
import { useLogSystem } from '../../composables/useLogSystem'
import { LIKELY_EMPTY_MESH_PREVIEW_ERROR } from '../../utils/AssetClassificationUtils'
import { PreviewContextMenuGesture } from '../../utils/PreviewContextMenuGesture'
import type { AssetPreviewRelation, AssetPreviewSection, PreviewAnimationClipRef } from '../../types'
import type { MeshPreviewTextureCandidate } from '../../composables/home/WorkspacePreviewUtils'
import PreviewDrawerPanel from './PreviewDrawerPanel.vue'

const Model3D = defineAsyncComponent(() => import('../Model3D.vue'))

const ws = useWorkspace()
const { addLog } = useLogSystem()
const { t } = useI18n()
const model3DRef = ref<any>(null)
const previewContextMenuVisible = ref(false)
const previewContextMenuX = ref(0)
const previewContextMenuY = ref(0)
const exportingPreviewResources = ref(false)
const textureScanLoading = ref(false)
const animatorScanLoading = ref(false)
const animationScanLoading = ref(false)
const animationApplyLoadingKey = ref('')
const loadedAnimationClipKey = ref('')
const playingAnimationClipKey = ref('')
const meshRegionSnapshot = ref<{ subMeshes: any[]; parts: any[] }>({ subMeshes: [], parts: [] })
const selectedMeshScope = ref<{ bundle_path: string; path_id: string; class_name: string; name?: string } | null>(null)
const previewContextMenuGesture = new PreviewContextMenuGesture()

function previewImageSrc(path: string | undefined) {
  if (!path) return ''
  return path.startsWith('data:') ? path : convertFileSrc(path)
}

const model3DGeometry = computed(() => {
  const geometry = ws.meshGeometryData.value
  if (!geometry) return null
  // Keep the original index of each candidate so the filtered view can still map
  // back to the exact raw source entry (filtering drops candidates without a png_path).
  const textureCandidates = (geometry.texture_candidates || [])
    .map((candidate, index) => ({ candidate, index }))
    .filter(({ candidate }) => Boolean(candidate.png_path))
    .map(({ candidate, index }) => ({
      ...candidate,
      png_path: previewImageSrc(candidate.png_path),
      candidate_index: index,
    }))
  return {
    ...geometry,
    diffuse_texture_path: previewImageSrc(geometry.diffuse_texture_path),
    glb_url: previewImageSrc(geometry.glb_path),
    texture_candidates: textureCandidates,
  }
})

const meshRegionControls = computed(() => {
  const subMeshes = meshRegionSnapshot.value.subMeshes
  if (subMeshes.length) {
    return subMeshes.map((item: any) => ({ ...item, kind: 'submesh' }))
  }
  return meshRegionSnapshot.value.parts.map((item: any) => ({ ...item, kind: 'part' }))
})

const textureCandidates = computed(() => model3DGeometry.value?.texture_candidates || [])
const animationNames = computed(() => model3DGeometry.value?.glb_animation_names || [])
const animators = computed(() => ws.previewAnimators.value || [])
const animationClips = computed(() => ws.previewAnimationClips.value || [])
const selectedAnimationClip = computed(() => ws.selectedPreviewAnimationClip.value)
const preferredAnimationName = computed(() => selectedAnimationClip.value?.name || '')
const canScanPreviewTextures = computed(() => {
  const asset = ws.previewTargetAsset.value
  return Boolean(asset && ['Mesh', 'Animator', 'GameObject'].includes(asset.class_name))
})
const canScanPreviewAnimators = computed(() => {
  const asset = ws.previewTargetAsset.value
  return Boolean(asset && [
    'Animator',
    'Animation',
    'GameObject',
    'Renderer',
    'MeshRenderer',
    'SkinnedMeshRenderer',
    'MeshFilter',
    'Transform',
  ].includes(asset.class_name))
})
const canScanPreviewAnimations = computed(() => {
  return animators.value.length > 0
})

function closePreviewContextMenu() {
  previewContextMenuVisible.value = false
}

function showPreviewContextMenu(event: MouseEvent) {
  if (!ws.previewTargetAsset.value) return
  event.preventDefault()
  if (!previewContextMenuGesture.shouldOpen(event)) {
    closePreviewContextMenu()
    return
  }
  previewContextMenuVisible.value = true
  previewContextMenuX.value = event.clientX
  previewContextMenuY.value = event.clientY
}

function handlePreviewPointerDown(event: PointerEvent) {
  previewContextMenuGesture.pointerDown(event)
}

function handlePreviewPointerMove(event: PointerEvent) {
  previewContextMenuGesture.pointerMove(event)
}

function handlePreviewPointerUp(event: PointerEvent) {
  previewContextMenuGesture.pointerUp(event)
}

function handlePreviewPointerCancel() {
  previewContextMenuGesture.pointerCancel()
}

function toggleMeshRegion(control: any) {
  const exposed = model3DRef.value
  if (!exposed) return
  if (control.kind === 'submesh') {
    selectedMeshScope.value = null
    ws.selectedPreviewMeshScope.value = null
    exposed.selectSingleSubMesh(control.index)
  } else {
    selectedMeshScope.value = control.bundlePath && control.pathId != null
      ? {
          bundle_path: control.bundlePath,
          path_id: control.pathId,
          class_name: control.className || 'Mesh',
          name: control.name || control.displayName,
        }
      : null
    ws.selectedPreviewMeshScope.value = selectedMeshScope.value
    exposed.selectSinglePart(control.key)
  }
  ws.previewAnimators.value = []
  ws.previewAnimationClips.value = []
  ws.selectedPreviewAnimationClip.value = null
}

function resetPreviewCameraPosition() {
  model3DRef.value?.resetCameraPosition?.()
  closePreviewContextMenu()
}

function toggleMeshRegionVisible(control: any, event: Event) {
  const exposed = model3DRef.value
  if (!exposed) return
  const checked = Boolean((event.target as HTMLInputElement | null)?.checked)
  if (control.kind === 'submesh') {
    exposed.setSubMeshVisible(control.index, checked)
  } else {
    exposed.togglePartVisible(control.key)
  }
}

function handleMeshRegionsChange(detail: { subMeshes: any[]; parts: any[] }) {
  meshRegionSnapshot.value = {
    subMeshes: detail.subMeshes || [],
    parts: detail.parts || [],
  }
  const selectedPart = (detail.parts || []).find((item: any) => item.selected && item.bundlePath && item.pathId != null)
  selectedMeshScope.value = selectedPart
    ? {
        bundle_path: selectedPart.bundlePath,
        path_id: selectedPart.pathId,
        class_name: selectedPart.className || 'Mesh',
        name: selectedPart.name || selectedPart.displayName,
      }
    : null
  ws.selectedPreviewMeshScope.value = selectedMeshScope.value
}

function applyTextureCandidate(candidate: MeshPreviewTextureCandidate) {
  const selection = model3DRef.value?.selectTextureCandidate(candidate)
  const geometry = ws.meshGeometryData.value
  if (!geometry) return
  const rawCandidate = typeof candidate.candidate_index === 'number'
    ? geometry.texture_candidates?.[candidate.candidate_index]
    : undefined
  const textureSourcePath = rawCandidate?.cache_png_path
    || rawCandidate?.png_path
    || candidate.cache_png_path
  ws.meshGeometryData.value = {
    ...geometry,
    diffuse_texture_path: textureSourcePath,
    diffuse_texture_path_id: rawCandidate?.texture_path_id || candidate.texture_path_id,
    diffuse_texture_bundle_path: rawCandidate?.texture_bundle_path || candidate.texture_bundle_path,
    diffuse_texture_name: rawCandidate?.texture_name || candidate.texture_name,
    diffuse_material_name: rawCandidate?.material_name || candidate.material_name,
    diffuse_material_index: rawCandidate?.material_index ?? candidate.material_index,
    diffuse_slot_name: rawCandidate?.slot_name || candidate.slot_name,
    diffuse_texture_user_selected: true,
    selected_submesh_indices: selection?.selected_submesh_indices ?? geometry.selected_submesh_indices,
  }
}

watch(
  () => `${ws.previewTargetAsset.value?.source_bundle_path || ''}:${ws.previewTargetAsset.value?.path_id || ''}:${ws.previewType.value}`,
  () => {
    meshRegionSnapshot.value = { subMeshes: [], parts: [] }
    selectedMeshScope.value = null
    ws.selectedPreviewMeshScope.value = null
    loadedAnimationClipKey.value = ''
    playingAnimationClipKey.value = ''
  },
)

async function scanTextures() {
  if (!canScanPreviewTextures.value || textureScanLoading.value) return
  textureScanLoading.value = true
  try {
    await ws.scanPreviewTextures()
  } finally {
    textureScanLoading.value = false
  }
}

async function scanAnimators() {
  if (!canScanPreviewAnimators.value || animatorScanLoading.value) return
  animatorScanLoading.value = true
  try {
    if (ws.selectedPreviewMeshScope.value) {
      const scope = ws.selectedPreviewMeshScope.value
      addLog(`  -> Current preview mesh scope: ${scope.class_name} '${scope.name || scope.path_id}' (${scope.bundle_path}:${scope.path_id})`, 'info')
    } else {
      addLog('  -> Current preview mesh scope: GameObject root', 'info')
    }
    await ws.scanPreviewAnimators()
  } finally {
    animatorScanLoading.value = false
  }
}

async function scanAnimations() {
  if (!canScanPreviewAnimations.value || animationScanLoading.value) return
  animationScanLoading.value = true
  try {
    if (ws.selectedPreviewMeshScope.value) {
      const scope = ws.selectedPreviewMeshScope.value
      addLog(`  -> Current preview animation scope: ${scope.class_name} '${scope.name || scope.path_id}' (${scope.bundle_path}:${scope.path_id})`, 'info')
    } else {
      addLog('  -> Current preview animation scope: GameObject root', 'info')
    }
    await ws.scanPreviewAnimations()
  } finally {
    animationScanLoading.value = false
  }
}

function animationClipKey(clip: PreviewAnimationClipRef) {
  return `${clip.bundle_path}:${clip.path_id}`
}

async function applyAnimationClip(clip: PreviewAnimationClipRef) {
  const key = animationClipKey(clip)
  if (animationApplyLoadingKey.value) return
  animationApplyLoadingKey.value = key
  try {
    await ws.applyPreviewAnimationClip(clip)
    if ((ws.meshGeometryData.value?.glb_animation_count || 0) > 0) {
      loadedAnimationClipKey.value = key
      playingAnimationClipKey.value = key
      const names = ws.meshGeometryData.value?.glb_animation_names || []
      addLog(`  -> Applied AnimationClip '${clip.name || `AnimationClip_${clip.path_id}`}': GLB returned ${names.length} playable animation(s)${names.length ? ` [${names.join(', ')}]` : ''}`, 'success')
    } else {
      loadedAnimationClipKey.value = ''
      playingAnimationClipKey.value = ''
      addLog('  -> Selected AnimationClip has no compatible playable channels for this preview model', 'warn')
    }
  } finally {
    animationApplyLoadingKey.value = ''
  }
}

async function toggleAnimationClipPlayback(clip: PreviewAnimationClipRef) {
  const key = animationClipKey(clip)
  const exposed = model3DRef.value
  if (playingAnimationClipKey.value === key) {
    exposed?.pauseAnimation?.()
    playingAnimationClipKey.value = ''
    return
  }
  if (loadedAnimationClipKey.value === key) {
    if ((ws.meshGeometryData.value?.glb_animation_count || 0) === 0) {
      loadedAnimationClipKey.value = ''
      playingAnimationClipKey.value = ''
      addLog('  -> Selected AnimationClip has no compatible playable channels for this preview model', 'warn')
      return
    }
    exposed?.resumeAnimation?.()
    playingAnimationClipKey.value = key
    return
  }

  await applyAnimationClip(clip)
}

function handleAnimationPlaybackChange(detail: { playing: boolean; clipIndex: number | null; clipName: string | null; targetedBoneNames?: string[] }) {
  const selected = selectedAnimationClip.value
  if (!selected) {
    loadedAnimationClipKey.value = ''
    playingAnimationClipKey.value = ''
    return
  }
  const key = animationClipKey(selected)
  if (detail.clipIndex != null) {
    loadedAnimationClipKey.value = key
    const requestedName = selected.name || `AnimationClip_${selected.path_id}`
    const actualName = detail.clipName || `Animation_${detail.clipIndex}`
    if (detail.playing) {
      addLog(`  -> AnimationClip playback matched: requested='${requestedName}', actual='${actualName}', index=${detail.clipIndex}`, 'info')
      const targets = detail.targetedBoneNames || []
      const onlyAttachmentTargets = targets.length > 0 && targets.every(name =>
        /weapon|body_dummy/i.test(name),
      )
      if (onlyAttachmentTargets) {
        addLog(`  -> This AnimationClip only targets attachment bones: ${targets.join(', ')}. Character body bones are not animated, so the model can appear static.`, 'warn')
      }
    }
  }
  playingAnimationClipKey.value = detail.playing ? key : ''
}

function clearPreviewDrawerCache(category: 'mesh' | 'texture' | 'animator' | 'animation') {
  ws.clearPreviewDrawerCache(category)
  if (category === 'animation') {
    loadedAnimationClipKey.value = ''
    playingAnimationClipKey.value = ''
  }
  if (category === 'mesh') {
    meshRegionSnapshot.value = { subMeshes: [], parts: [] }
    selectedMeshScope.value = null
  }
}

function safeFileStem(value: string) {
  return (value || 'preview')
    .replace(/[<>:"/\\|?*\u0000-\u001F]/g, '_')
    .replace(/\s+/g, '_')
    .slice(0, 80)
}

async function collectPreviewResourcePaths(): Promise<string[]> {
  const paths: string[] = []
  const geometry = ws.meshGeometryData.value
  if (geometry?.glb_path) paths.push(geometry.glb_path)
  if (ws.previewType.value === 'mesh' && ws.previewTargetAsset.value?.class_name === 'Mesh' && !geometry?.glb_path) {
    const bundlePath = ws.findBundleForAsset(ws.previewTargetAsset.value)
    if (bundlePath) {
      const cacheDir = await ws.getCacheDir()
      const glbPath = await invoke<string>('export_mesh_preview', {
        bundlePath,
        pathId: ws.previewTargetAsset.value.path_id,
        cacheDir,
      })
      if (glbPath) paths.push(glbPath)
    }
  }
  for (const candidate of geometry?.texture_candidates || []) {
    const texturePath = candidate.cache_png_path || candidate.png_path
    if (texturePath && !texturePath.startsWith('data:')) {
      paths.push(texturePath)
    }
  }
  if (ws.previewType.value === 'texture' && ws.previewFilePath.value && !ws.previewFilePath.value.startsWith('data:')) {
    paths.push(ws.previewFilePath.value)
  }
  if (ws.previewType.value === 'texture' && ws.textureInfo.value?.png_path) {
    paths.push(ws.textureInfo.value.png_path)
  }
  return [...new Set(paths)]
}

async function exportCurrentPreviewResources() {
  closePreviewContextMenu()
  if (exportingPreviewResources.value) return
  const outputDir = await open({
    directory: true,
    multiple: false,
    title: 'Export current preview resources',
  })
  if (!outputDir || typeof outputDir !== 'string') return

  exportingPreviewResources.value = true
  try {
    const paths = await collectPreviewResourcePaths()
    if (!paths.length) {
      addLog('No loaded preview resources to export', 'warn')
      return
    }
    await mkdir(outputDir, { recursive: true })
    const asset = ws.previewTargetAsset.value
    const stem = safeFileStem(asset?.name || asset?.path || asset?.path_id || 'preview')
    const copied: string[] = []
    for (let index = 0; index < paths.length; index++) {
      const source = paths[index]
      const sourceBase = await basename(source)
      const target = await join(outputDir, `${stem}_${index + 1}_${sourceBase}`)
      await copyFile(source, target)
      copied.push(target)
    }
    addLog(`Exported ${copied.length} loaded preview resource(s)`, 'success')
    if (copied.length === 1) {
      await revealItemInDir(copied[0])
    } else {
      await openPath(outputDir)
    }
  } catch (error) {
    addLog(`Export current preview resources failed: ${error}`, 'error')
  } finally {
    exportingPreviewResources.value = false
  }
}

function relationTitle(relation: AssetPreviewRelation) {
  const dir = relation.direction === 'in' ? '<-' : '->'
  const name = relation.name || relation.class_name || relation.path_id
  return `${dir} ${relation.relation_type} ${name}`
}

function relationSubtitle(relation: AssetPreviewRelation) {
  const field = relation.field_path || relation.relation_type
  const type = relation.class_name || 'Unknown'
  return `${field} | ${type} | ${relation.path_id}`
}

function typedSectionRows(section: AssetPreviewSection) {
  return section.rows || []
}
</script>

<template>
  <!-- Placeholder when no asset is selected -->
  <div v-if="!ws.previewTargetAsset.value" class="panel-empty preview-empty">
    {{ t('assetPreview.selectAsset') }}
  </div>

  <!-- Preview loading -->
  <div v-else-if="ws.previewLoading.value" class="preview-loading">
    <el-icon class="is-loading preview-refresh-loading" :size="32"><Refresh /></el-icon>
    <p>{{ t('assetPreview.exportingPreview') }}</p>
  </div>

  <div
    v-else-if="ws.previewType.value === 'mesh'"
    class="mesh-preview-shell"
    @pointerdown="handlePreviewPointerDown"
    @pointermove="handlePreviewPointerMove"
    @pointerup="handlePreviewPointerUp"
    @pointercancel="handlePreviewPointerCancel"
    @contextmenu.prevent="showPreviewContextMenu"
  >
    <!-- Mesh preview error -->
    <div v-if="ws.previewError.value" class="unknown-preview">
      <el-icon :size="48"><VideoCamera /></el-icon>
      <template v-if="ws.previewError.value === LIKELY_EMPTY_MESH_PREVIEW_ERROR">
        <p>{{ t('assetPreview.likelyEmptyMeshTitle') }}</p>
        <p class="preview-empty-mesh-detail">{{ t('assetPreview.likelyEmptyMeshDetail') }}</p>
      </template>
      <template v-else>
        <p>{{ t('assetPreview.meshUnavailableTitle') }}</p>
        <p class="preview-error-detail">{{ ws.previewError.value }}</p>
      </template>
    </div>

    <!-- Mesh 3D preview (geometry data passed directly, Three.js builds BufferGeometry) -->
    <div v-else-if="ws.meshGeometryData.value" class="mesh-preview-layout">
      <Model3D
        ref="model3DRef"
        :geometry-data="model3DGeometry"
        :asset-name="ws.previewTargetAsset.value.path"
        :preferred-animation-name="preferredAnimationName"
        @geometry-error="(d: any) => addLog(t('assetPreview.indexFallback', { maxIdx: d.maxIdx, vertexCount: d.vertexCount, indexCount: d.indexCount }), 'warn')"
        @mesh-regions-change="handleMeshRegionsChange"
        @animation-playback-change="handleAnimationPlaybackChange"
      />

      <PreviewDrawerPanel
        :model-geometry="model3DGeometry"
        :mesh-region-controls="meshRegionControls"
        :texture-candidates="textureCandidates"
        :animators="animators"
        :animation-clips="animationClips"
        :animation-names="animationNames"
        :selected-animation-clip="selectedAnimationClip"
        :can-scan-preview-textures="canScanPreviewTextures"
        :can-scan-preview-animators="canScanPreviewAnimators"
        :can-scan-preview-animations="canScanPreviewAnimations"
        :texture-scan-loading="textureScanLoading"
        :animator-scan-loading="animatorScanLoading"
        :animation-scan-loading="animationScanLoading"
        :animation-apply-loading-key="animationApplyLoadingKey"
        :playing-animation-clip-key="playingAnimationClipKey"
        @toggle-mesh-region="toggleMeshRegion"
        @toggle-mesh-region-visible="({ region, event }) => toggleMeshRegionVisible(region, event)"
        @apply-texture-candidate="applyTextureCandidate"
        @scan-textures="scanTextures"
        @scan-animators="scanAnimators"
        @scan-animations="scanAnimations"
        @toggle-animation-clip="toggleAnimationClipPlayback"
        @clear-preview-cache="clearPreviewDrawerCache"
      />
    </div>
  </div>

    <!-- Texture preview: image on the left, metadata on the right -->
    <div
      v-else-if="ws.previewType.value === 'texture' && ws.previewFilePath.value"
      class="texture-preview"
      @pointerdown="handlePreviewPointerDown"
      @pointermove="handlePreviewPointerMove"
      @pointerup="handlePreviewPointerUp"
      @pointercancel="handlePreviewPointerCancel"
      @contextmenu.prevent="showPreviewContextMenu"
    >
      <div class="texture-stage">
        <div class="texture-frame">
          <img :src="previewImageSrc(ws.previewFilePath.value)" class="texture-image" />
        </div>
      </div>
      <div v-if="ws.textureInfo.value" class="texture-details">
        <div class="texture-detail-heading">
          <span>{{ ws.textureInfo.value.width }} x {{ ws.textureInfo.value.height }}</span>
          <strong>{{ ws.textureInfo.value.texture_format_name }}</strong>
          <em :class="{ 'has-alpha': ws.textureInfo.value.has_alpha }">
            {{ ws.textureInfo.value.has_alpha ? 'Alpha' : t('assetPreview.noAlpha') }}
          </em>
        </div>
        <div class="texture-detail-row">
          <span>{{ t('assetPreview.fullSize') }}</span>
          <strong>{{ ws.textureInfo.value.complete_image_size }} B</strong>
        </div>
        <div class="texture-detail-row">
          <span>{{ t('assetPreview.imageCount') }}</span>
          <strong>{{ ws.textureInfo.value.image_count }}</strong>
        </div>
        <div class="texture-detail-row">
          <span>{{ t('assetPreview.colorSpace') }}</span>
          <strong>{{ ws.textureInfo.value.color_space_name }}</strong>
        </div>
        <div class="texture-detail-row">
          <span>{{ t('assetPreview.filter') }}</span>
          <strong>{{ ws.textureInfo.value.filter_mode_name }}</strong>
        </div>
        <div class="texture-detail-row">
          <span>{{ t('assetPreview.wrap') }}</span>
          <strong>{{ ws.textureInfo.value.wrap_mode_name }}</strong>
        </div>
        <div class="texture-detail-row">
          <span>{{ t('assetPreview.anisotropy') }}</span>
          <strong>{{ ws.textureInfo.value.aniso }}</strong>
        </div>
        <div class="texture-detail-row">
          <span>{{ t('assetPreview.readable') }}</span>
          <strong>{{ ws.textureInfo.value.is_readable ? t('common.yes') : t('common.no') }}</strong>
        </div>
        <div class="texture-detail-row">
          <span>{{ t('assetPreview.mipStream') }}</span>
          <strong>{{ ws.textureInfo.value.streaming_mipmaps ? t('common.yes') : t('common.no') }}</strong>
        </div>
        <div class="texture-detail-row">
          <span>{{ t('assetPreview.mipCount') }}</span>
          <strong>{{ ws.textureInfo.value.mip_count > 0 ? ws.textureInfo.value.mip_count : t('assetPreview.noMips') }}</strong>
        </div>
        <div class="texture-detail-row">
          <span>{{ ws.textureInfo.value.texture_dimension_name }}</span>
          <strong>{{ (ws.textureInfo.value.byte_size / 1024).toFixed(1) }} KB</strong>
        </div>
      </div>
    </div>

    <!-- Texture preview (error state) -->
    <div
      v-else-if="ws.previewType.value === 'texture' && ws.previewError.value"
      class="texture-preview texture-preview-center"
      @pointerdown="handlePreviewPointerDown"
      @pointermove="handlePreviewPointerMove"
      @pointerup="handlePreviewPointerUp"
      @pointercancel="handlePreviewPointerCancel"
      @contextmenu.prevent="showPreviewContextMenu"
    >
      <div class="texture-frame">
        <div class="texture-placeholder">
          <el-icon :size="48"><PictureFilled /></el-icon>
          <p>{{ ws.previewError.value }}</p>
        </div>
      </div>
    </div>

    <!-- Texture preview (empty state) -->
    <div
      v-else-if="ws.previewType.value === 'texture'"
      class="texture-preview texture-preview-center"
      @pointerdown="handlePreviewPointerDown"
      @pointermove="handlePreviewPointerMove"
      @pointerup="handlePreviewPointerUp"
      @pointercancel="handlePreviewPointerCancel"
      @contextmenu.prevent="showPreviewContextMenu"
    >
      <div class="texture-frame">
        <div class="texture-placeholder">
          <el-icon :size="48"><PictureFilled /></el-icon>
          <p>{{ t('assetPreview.clickTexture') }}</p>
        </div>
      </div>
    </div>

    <!-- Typed structured preview -->
    <div
      v-else-if="ws.previewType.value === 'typed'"
      class="typed-preview"
      @pointerdown="handlePreviewPointerDown"
      @pointermove="handlePreviewPointerMove"
      @pointerup="handlePreviewPointerUp"
      @pointercancel="handlePreviewPointerCancel"
      @contextmenu.prevent="showPreviewContextMenu"
    >
      <template v-if="ws.typedPreviewData.value">
        <div class="typed-preview-header">
          <div>
            <h3>{{ ws.typedPreviewData.value.name || ws.previewTargetAsset.value.name || ws.previewTargetAsset.value.path }}</h3>
            <p>{{ ws.typedPreviewData.value.class_name }} | PathID {{ ws.typedPreviewData.value.path_id }}</p>
          </div>
          <span>{{ ws.typedPreviewData.value.unity_version }}</span>
        </div>

        <div v-if="ws.typedPreviewData.value.warnings.length" class="typed-warning-list">
          <div v-for="warning in ws.typedPreviewData.value.warnings" :key="warning" class="typed-warning">
            {{ warning }}
          </div>
        </div>

        <div class="typed-section-list">
          <section
            v-for="section in ws.typedPreviewData.value.sections"
            :key="`${section.title}:${section.kind}`"
            class="typed-section"
          >
            <h4>{{ section.title }}</h4>
            <div class="typed-row-list">
              <div
                v-for="row in typedSectionRows(section)"
                :key="`${section.title}:${row.label}:${row.value}`"
                class="typed-row"
              >
                <span>{{ row.label }}</span>
                <strong>{{ row.value }}</strong>
              </div>
            </div>
          </section>

          <section v-if="ws.typedPreviewData.value.relations.length" class="typed-section">
            <h4>Relations</h4>
            <div class="relation-list">
              <div
                v-for="relation in ws.typedPreviewData.value.relations.slice(0, 160)"
                :key="`${relation.direction}:${relation.relation_type}:${relation.bundle_path}:${relation.path_id}:${relation.field_path}`"
                class="relation-row"
              >
                <span>{{ relationTitle(relation) }}</span>
                <strong>{{ relationSubtitle(relation) }}</strong>
              </div>
            </div>
          </section>
        </div>
      </template>
      <div v-else class="unknown-preview">
        <el-icon :size="48"><Document /></el-icon>
        <p>{{ ws.previewError.value || t('assetPreview.unsupportedForType', { type: ws.previewTargetAsset.value.class_name }) }}</p>
      </div>
    </div>

    <!-- Unsupported preview type -->
    <div v-else class="unknown-preview">
      <el-icon :size="48"><Document /></el-icon>
      <p>{{ t('assetPreview.unsupportedForType', { type: ws.previewTargetAsset.value.class_name }) }}</p>
    </div>

    <Teleport to="body">
      <div
        v-if="previewContextMenuVisible"
        class="preview-context-menu"
        :style="{ left: previewContextMenuX + 'px', top: previewContextMenuY + 'px' }"
        @click.stop
      >
        <button
          v-if="ws.previewType.value === 'mesh'"
          type="button"
          class="preview-context-menu-item"
          @click="resetPreviewCameraPosition"
        >
          重置摄像机位置
        </button>
        <button
          type="button"
          class="preview-context-menu-item"
          :disabled="exportingPreviewResources"
          @click="exportCurrentPreviewResources"
        >
          Export Current Preview Resources
        </button>
        <button type="button" class="preview-context-menu-item" @click="closePreviewContextMenu">
          Cancel
        </button>
      </div>
      <div
        v-if="previewContextMenuVisible"
        class="preview-context-menu-backdrop"
        @click="closePreviewContextMenu"
      ></div>
    </Teleport>
</template>

<style scoped>
.mesh-preview-shell {
  position: relative;
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  align-self: stretch;
  min-height: 0;
}

.mesh-preview-layout {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  width: 100%;
  height: 100%;
  min-height: 0;
  overflow: hidden;
  background: #2f2f2f;
}

.preview-context-menu-backdrop {
  position: fixed;
  inset: 0;
  z-index: 3998;
}

.preview-context-menu {
  position: fixed;
  z-index: 3999;
  min-width: 210px;
  padding: 5px;
  border: 1px solid var(--app-border-soft);
  border-radius: 6px;
  background: var(--app-surface);
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.2);
}

.preview-context-menu-item {
  width: 100%;
  height: 28px;
  display: flex;
  align-items: center;
  padding: 0 8px;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: var(--app-text-primary);
  cursor: pointer;
  font: inherit;
  font-size: 12px;
  text-align: left;
}

.preview-context-menu-item:hover:not(:disabled) {
  background: var(--app-surface-muted);
}

.preview-context-menu-item:disabled {
  color: var(--el-text-color-disabled);
  cursor: not-allowed;
}

.preview-empty-mesh-detail {
  max-width: min(520px, 86%);
  margin-top: 4px;
  color: var(--el-text-color-secondary);
  font-size: 12px;
  line-height: 1.7;
  text-align: center;
}
</style>

<style scoped>
.typed-preview {
  width: 100%;
  height: 100%;
  overflow: auto;
  padding: 14px;
  background: var(--el-bg-color);
}

.typed-preview-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--el-border-color-light);
}

.typed-preview-header h3 {
  margin: 0;
  color: var(--el-text-color-primary);
  font-size: 15px;
  line-height: 1.35;
}

.typed-preview-header p,
.typed-preview-header span {
  margin: 4px 0 0;
  color: var(--el-text-color-secondary);
  font-size: 12px;
  line-height: 1.4;
}

.typed-warning-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 10px;
}

.typed-warning {
  padding: 7px 8px;
  border: 1px solid color-mix(in srgb, var(--app-accent-warm) 35%, transparent);
  border-radius: 6px;
  background: color-mix(in srgb, var(--app-accent-warm) 8%, transparent);
  color: var(--el-text-color-primary);
  font-size: 12px;
}

.typed-section-list {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-top: 12px;
}

.typed-section {
  padding-bottom: 10px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}

.typed-section h4 {
  margin: 0 0 8px;
  color: var(--el-text-color-primary);
  font-size: 13px;
  line-height: 1.3;
}

.typed-row-list,
.relation-list {
  display: grid;
  gap: 4px;
}

.typed-row,
.relation-row {
  display: grid;
  grid-template-columns: minmax(96px, 34%) minmax(0, 1fr);
  gap: 10px;
  min-height: 24px;
  align-items: start;
  padding: 4px 6px;
  border-radius: 4px;
  background: var(--el-fill-color-lighter);
}

.typed-row span,
.relation-row span {
  min-width: 0;
  overflow-wrap: anywhere;
  color: var(--el-text-color-secondary);
  font-size: 12px;
  line-height: 1.35;
}

.typed-row strong,
.relation-row strong {
  min-width: 0;
  overflow-wrap: anywhere;
  color: var(--el-text-color-primary);
  font-size: 12px;
  font-weight: 600;
  line-height: 1.35;
}

.relation-row {
  grid-template-columns: minmax(120px, 42%) minmax(0, 1fr);
}
</style>
