<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { Delete, Refresh, VideoPause, VideoPlay } from '@element-plus/icons-vue'
import type { PreviewAnimationClipRef, PreviewAnimatorRef } from '../../types'
import type { MeshPreviewTextureCandidate } from '../../composables/home/WorkspacePreviewUtils'

type MeshRegionControl = {
  key: string
  kind: 'submesh' | 'part'
  index: number
  name: string
  displayName: string
  color: string
  visible: boolean
  selected: boolean
}

const props = defineProps<{
  modelGeometry: any
  meshRegionControls: MeshRegionControl[]
  textureCandidates: Array<MeshPreviewTextureCandidate & { candidate_index?: number }>
  animators: PreviewAnimatorRef[]
  animationClips: PreviewAnimationClipRef[]
  animationNames: string[]
  selectedAnimationClip: PreviewAnimationClipRef | null
  canScanPreviewTextures: boolean
  canScanPreviewAnimators: boolean
  canScanPreviewAnimations: boolean
  textureScanLoading: boolean
  animatorScanLoading: boolean
  animationScanLoading: boolean
  animationApplyLoadingKey: string
  playingAnimationClipKey: string
}>()

const emit = defineEmits<{
  (e: 'toggle-mesh-region', region: MeshRegionControl): void
  (e: 'toggle-mesh-region-visible', payload: { region: MeshRegionControl; event: Event }): void
  (e: 'apply-texture-candidate', candidate: MeshPreviewTextureCandidate): void
  (e: 'scan-textures'): void
  (e: 'scan-animators'): void
  (e: 'scan-animations'): void
  (e: 'toggle-animation-clip', clip: PreviewAnimationClipRef): void
  (e: 'clear-preview-cache', category: 'mesh' | 'texture' | 'animator' | 'animation'): void
}>()

const meshDrawerOpen = ref(true)
const textureDrawerOpen = ref(true)
const animatorDrawerOpen = ref(true)
const animationDrawerOpen = ref(true)
const panelWidth = ref(300)
const resizeHandlers = ref<{ move: (e: PointerEvent) => void; up: () => void } | null>(null)

const panelStyle = computed(() => ({
  width: `${panelWidth.value}px`,
}))

function beginResize(event: PointerEvent) {
  const startX = event.clientX
  const startWidth = panelWidth.value
  const pointerId = event.pointerId
  const target = event.currentTarget as HTMLElement
  target.setPointerCapture(pointerId)

  const move = (moveEvent: PointerEvent) => {
    const next = startWidth - (moveEvent.clientX - startX)
    panelWidth.value = Math.min(520, Math.max(220, next))
  }
  const up = () => {
    target.releasePointerCapture(pointerId)
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', up)
    window.removeEventListener('pointercancel', up)
  }

  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', up, { once: true })
  window.addEventListener('pointercancel', up, { once: true })
  resizeHandlers.value = { move, up }
}

// If the drawer unmounts mid-drag (e.g. preview switches away from the mesh
// tab), pointerup/pointercancel never fire and the pointermove listener would
// keep writing to the destroyed component. Clean it up on unmount.
onBeforeUnmount(() => {
  const handlers = resizeHandlers.value
  if (handlers) {
    window.removeEventListener('pointermove', handlers.move)
    window.removeEventListener('pointerup', handlers.up)
    window.removeEventListener('pointercancel', handlers.up)
  }
})

function animationClipKey(clip: PreviewAnimationClipRef) {
  return `${clip.bundle_path}:${clip.path_id}`
}

function animationClipTitle(clip: PreviewAnimationClipRef) {
  return `${clip.name || 'AnimationClip'}\n${clip.bundle_path}\nPathID ${clip.path_id}`
}

function animatorTitle(animator: PreviewAnimatorRef) {
  return `${animator.name || animator.class_name}\n${animator.bundle_path}\nPathID ${animator.path_id}`
}

function fileNameFromPath(path: string) {
  return path.split(/[\\/]/).pop() || path
}

function formatBytes(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) return ''
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${bytes} B`
}

function isSelectedAnimationClip(clip: PreviewAnimationClipRef) {
  return props.selectedAnimationClip
    && props.selectedAnimationClip.bundle_path === clip.bundle_path
    && props.selectedAnimationClip.path_id === clip.path_id
}
</script>

<template>
  <aside class="preview-drawer-panel" :style="panelStyle">
    <div
      class="preview-drawer-resize-handle"
      role="separator"
      aria-orientation="vertical"
      title="Resize panel"
      @pointerdown.prevent.stop="beginResize"
    ></div>

    <section class="preview-drawer">
      <div class="preview-drawer-header">
        <button type="button" class="preview-drawer-toggle" @click="meshDrawerOpen = !meshDrawerOpen">
          <span>{{ meshRegionControls.length ? 'Submesh List' : 'Mesh' }}</span>
          <strong>{{ meshRegionControls.length || (modelGeometry?.glb_mesh_count || 1) }}</strong>
        </button>
        <button type="button" class="preview-cache-clear-button" title="Clear Mesh cache" @click.stop="emit('clear-preview-cache', 'mesh')">
          <el-icon :size="14"><Delete /></el-icon>
        </button>
      </div>
      <div v-if="meshDrawerOpen" class="preview-drawer-body">
        <template v-if="meshRegionControls.length">
          <button
            v-for="region in meshRegionControls"
            :key="region.key"
            type="button"
            class="preview-list-row"
            :class="{ selected: region.selected, muted: !region.visible }"
            :title="region.name"
            @click.stop="emit('toggle-mesh-region', region)"
          >
            <input
              type="checkbox"
              :checked="region.visible"
              @click.stop
              @change="emit('toggle-mesh-region-visible', { region, event: $event })"
            />
            <span class="preview-row-swatch" :style="{ background: region.color }"></span>
            <span class="preview-row-label">{{ region.displayName }}</span>
          </button>
        </template>
        <div v-else class="preview-drawer-empty">
          {{ modelGeometry?.vertex_count || 0 }} vertices / {{ modelGeometry?.triangle_count || 0 }} triangles
        </div>
      </div>
    </section>

    <section class="preview-drawer">
      <div class="preview-drawer-header">
        <button type="button" class="preview-drawer-toggle" @click="textureDrawerOpen = !textureDrawerOpen">
          <span>Texture List</span>
          <strong>{{ textureCandidates.length }}</strong>
        </button>
        <button type="button" class="preview-cache-clear-button" title="Clear Texture cache" @click.stop="emit('clear-preview-cache', 'texture')">
          <el-icon :size="14"><Delete /></el-icon>
        </button>
      </div>
      <div v-if="textureDrawerOpen" class="preview-drawer-body">
        <el-button
          v-if="canScanPreviewTextures && textureCandidates.length === 0"
          size="small"
          type="primary"
          plain
          class="preview-scan-button"
          :loading="textureScanLoading"
          @click.stop="emit('scan-textures')"
        >
          Scan Textures
        </el-button>
        <button
          v-for="candidate in textureCandidates"
          :key="`${candidate.texture_bundle_path}:${candidate.texture_path_id}:${candidate.material_index}:${candidate.slot_name}`"
          type="button"
          class="preview-texture-row"
          :title="`${candidate.texture_name}\n${candidate.material_name || ''}\n${candidate.slot_name || ''}`"
          @click.stop="emit('apply-texture-candidate', candidate)"
        >
          <img class="preview-texture-thumb" :src="candidate.png_path" :alt="candidate.texture_name" />
          <span class="preview-texture-body">
            <span class="preview-texture-name">{{ candidate.texture_name || fileNameFromPath(candidate.png_path) }}</span>
            <span class="preview-texture-meta">{{ candidate.width || 0 }} x {{ candidate.height || 0 }} | {{ candidate.texture_format_name || '?' }}</span>
            <span class="preview-texture-meta">{{ candidate.usage || '?' }}<template v-if="candidate.slot_name"> | {{ candidate.slot_name }}</template></span>
          </span>
        </button>
        <div v-if="!canScanPreviewTextures && textureCandidates.length === 0" class="preview-drawer-empty">
          No texture scan available
        </div>
      </div>
    </section>

    <section class="preview-drawer">
      <div class="preview-drawer-header">
        <button type="button" class="preview-drawer-toggle" @click="animatorDrawerOpen = !animatorDrawerOpen">
          <span>Animator</span>
          <strong>{{ animators.length }}</strong>
        </button>
        <button type="button" class="preview-cache-clear-button" title="Clear Animator cache" @click.stop="emit('clear-preview-cache', 'animator')">
          <el-icon :size="14"><Delete /></el-icon>
        </button>
      </div>
      <div v-if="animatorDrawerOpen" class="preview-drawer-body">
        <el-button
          v-if="canScanPreviewAnimators && animators.length === 0"
          size="small"
          type="primary"
          plain
          class="preview-scan-button"
          :loading="animatorScanLoading"
          @click.stop="emit('scan-animators')"
        >
          Scan Animator
        </el-button>
        <button
          v-for="animator in animators"
          :key="`${animator.bundle_path}:${animator.path_id}`"
          type="button"
          class="preview-list-row"
          :title="animatorTitle(animator)"
        >
          <span class="preview-row-badge preview-row-badge-animator">A</span>
          <span class="preview-row-label">{{ animator.name || `${animator.class_name}_${animator.path_id}` }}</span>
          <small>{{ animator.class_name }} | {{ animator.path_id }}</small>
        </button>
        <div v-if="!canScanPreviewAnimators && animators.length === 0" class="preview-drawer-empty">
          No Animator scan available
        </div>
      </div>
    </section>

    <section class="preview-drawer">
      <div class="preview-drawer-header">
        <button type="button" class="preview-drawer-toggle" @click="animationDrawerOpen = !animationDrawerOpen">
          <span>Animation Clips</span>
          <strong>{{ animationClips.length || animationNames.length }}</strong>
        </button>
        <button type="button" class="preview-cache-clear-button" title="Clear Animation cache" @click.stop="emit('clear-preview-cache', 'animation')">
          <el-icon :size="14"><Delete /></el-icon>
        </button>
      </div>
      <div v-if="animationDrawerOpen" class="preview-drawer-body">
        <el-button
          v-if="canScanPreviewAnimations && animationClips.length === 0 && animationNames.length === 0"
          size="small"
          type="primary"
          plain
          class="preview-scan-button"
          :loading="animationScanLoading"
          @click.stop="emit('scan-animations')"
        >
          Scan Animations
        </el-button>
        <div
          v-for="clip in animationClips"
          :key="`${clip.bundle_path}:${clip.path_id}`"
          class="preview-list-row preview-animation-row"
          :class="{
            selected: isSelectedAnimationClip(clip),
            disabled: Boolean(animationApplyLoadingKey),
          }"
          role="button"
          tabindex="0"
          :aria-disabled="Boolean(animationApplyLoadingKey)"
          :title="animationClipTitle(clip)"
          @click.stop="!animationApplyLoadingKey && emit('toggle-animation-clip', clip)"
          @keydown.enter.stop.prevent="!animationApplyLoadingKey && emit('toggle-animation-clip', clip)"
          @keydown.space.stop.prevent="!animationApplyLoadingKey && emit('toggle-animation-clip', clip)"
        >
          <button
            type="button"
            class="preview-clip-play-button"
            :title="playingAnimationClipKey === animationClipKey(clip) ? 'Pause AnimationClip' : 'Play AnimationClip'"
            :disabled="Boolean(animationApplyLoadingKey)"
            @click.stop="emit('toggle-animation-clip', clip)"
          >
            <el-icon v-if="animationApplyLoadingKey === animationClipKey(clip)" class="is-loading" :size="14"><Refresh /></el-icon>
            <el-icon v-else :size="14">
              <VideoPause v-if="playingAnimationClipKey === animationClipKey(clip)" />
              <VideoPlay v-else />
            </el-icon>
          </button>
          <span class="preview-animation-body">
            <span class="preview-animation-name">{{ clip.name || `AnimationClip_${clip.path_id}` }}</span>
            <small class="preview-animation-meta">
              <template v-if="animationApplyLoadingKey === animationClipKey(clip)">Loading</template>
              <template v-else>{{ clip.path_id }}<template v-if="clip.byte_size"> | {{ formatBytes(clip.byte_size) }}</template></template>
            </small>
          </span>
        </div>
        <div v-if="animationClips.length === 0 && animationNames.length === 0 && !canScanPreviewAnimations" class="preview-drawer-empty">
          Scan Animator first
        </div>
        <div v-else-if="animationClips.length === 0 && animationNames.length === 0" class="preview-drawer-empty">
          Not scanned
        </div>
        <template v-if="animationClips.length === 0">
          <div
            v-for="name in animationNames"
            :key="name"
            class="preview-list-row preview-animation-row"
          >
            <span class="preview-animation-body">
              <span class="preview-animation-name">{{ name }}</span>
            </span>
          </div>
        </template>
      </div>
    </section>
  </aside>
</template>

<style scoped>
.preview-drawer-panel {
  position: relative;
  min-width: 220px;
  max-width: 520px;
  height: 100%;
  overflow-y: auto;
  border-left: 1px solid rgba(255, 255, 255, 0.12);
  background: color-mix(in srgb, var(--app-surface) 92%, #20242b);
}

.preview-drawer-resize-handle {
  position: absolute;
  top: 0;
  bottom: 0;
  left: 0;
  z-index: 2;
  width: 6px;
  cursor: col-resize;
  background: linear-gradient(
    to right,
    color-mix(in srgb, var(--el-color-primary) 42%, transparent),
    transparent
  );
}

.preview-drawer-resize-handle:hover {
  background: color-mix(in srgb, var(--el-color-primary) 36%, transparent);
}

.preview-drawer {
  border-bottom: 1px solid var(--app-border-soft);
}

.preview-drawer-header {
  width: 100%;
  height: 34px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 0 10px;
  border: 0;
  background: var(--app-surface-soft);
  color: var(--app-text-primary);
  font: inherit;
  font-size: 12px;
  font-weight: 700;
  text-align: left;
}

.preview-drawer-header:hover,
.preview-drawer-header:has(.preview-drawer-toggle:hover) {
  background: var(--app-surface-muted);
}

.preview-drawer-header strong {
  color: var(--app-text-secondary);
  font-size: 11px;
}

.preview-drawer-toggle {
  min-width: 0;
  height: 100%;
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
  cursor: pointer;
  font: inherit;
  font-weight: inherit;
  text-align: left;
}

.preview-drawer-toggle span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.preview-cache-clear-button {
  width: 24px;
  height: 24px;
  flex: 0 0 24px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 5px;
  background: transparent;
  color: var(--el-color-danger);
  cursor: pointer;
}

.preview-cache-clear-button:hover {
  border-color: color-mix(in srgb, var(--el-color-danger) 36%, transparent);
  background: color-mix(in srgb, var(--el-color-danger) 12%, transparent);
}

.preview-drawer-body {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 6px;
}

.preview-list-row,
.preview-texture-row {
  width: 100%;
  min-height: 28px;
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 5px 6px;
  border: 1px solid transparent;
  border-radius: 5px;
  background: transparent;
  color: var(--app-text-primary);
  cursor: pointer;
  font: inherit;
  font-size: 12px;
  text-align: left;
}

.preview-list-row:hover,
.preview-texture-row:hover,
.preview-list-row.selected {
  border-color: color-mix(in srgb, var(--el-color-primary) 35%, transparent);
  background: color-mix(in srgb, var(--el-color-primary) 9%, transparent);
}

.preview-list-row:disabled,
.preview-list-row.disabled {
  cursor: wait;
  opacity: 0.72;
}

.preview-list-row.muted {
  opacity: 0.58;
}

.preview-list-row input {
  flex: 0 0 auto;
}

.preview-row-swatch {
  width: 10px;
  height: 10px;
  flex: 0 0 auto;
  border-radius: 50%;
}

.preview-row-badge {
  flex: 0 0 auto;
  min-width: 20px;
  padding: 1px 4px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--el-color-warning) 18%, transparent);
  color: var(--el-color-warning);
  font-size: 10px;
  font-weight: 700;
  text-align: center;
}

.preview-row-badge-animator {
  background: color-mix(in srgb, var(--el-color-primary) 18%, transparent);
  color: var(--el-color-primary);
}

.preview-row-label,
.preview-list-row small {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.preview-list-row small {
  flex: 0 0 auto;
  max-width: 92px;
  margin-left: auto;
  color: var(--app-text-secondary);
  font-size: 10px;
}

.preview-animation-row {
  align-items: flex-start;
}

.preview-animation-body {
  min-width: 0;
  display: flex;
  flex: 1 1 auto;
  flex-direction: column;
  gap: 2px;
}

.preview-animation-name,
.preview-animation-meta {
  min-width: 0;
  overflow: visible;
  text-overflow: clip;
  white-space: normal;
  overflow-wrap: anywhere;
  word-break: break-word;
}

.preview-animation-name {
  line-height: 1.28;
}

.preview-animation-meta {
  max-width: none;
  margin-left: 0;
  line-height: 1.25;
}

.preview-clip-play-button {
  width: 24px;
  height: 24px;
  flex: 0 0 24px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 1px solid color-mix(in srgb, var(--el-color-primary) 28%, transparent);
  border-radius: 50%;
  background: color-mix(in srgb, var(--el-color-primary) 10%, transparent);
  color: var(--el-color-primary);
  cursor: pointer;
}

.preview-clip-play-button:hover:not(:disabled) {
  background: color-mix(in srgb, var(--el-color-primary) 18%, transparent);
}

.preview-clip-play-button:disabled {
  cursor: wait;
  opacity: 0.68;
}

.preview-scan-button {
  width: 100%;
}

.preview-drawer-empty {
  padding: 8px 6px;
  color: var(--app-text-secondary);
  font-size: 12px;
  line-height: 1.45;
}

.preview-texture-row {
  align-items: flex-start;
}

.preview-texture-thumb {
  width: 34px;
  height: 34px;
  flex: 0 0 auto;
  border-radius: 4px;
  object-fit: cover;
  background: var(--el-fill-color-darker);
}

.preview-texture-body {
  display: flex;
  flex-direction: column;
  min-width: 0;
  gap: 1px;
}

.preview-texture-name,
.preview-texture-meta {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.preview-texture-name {
  color: var(--app-text-primary);
  font-weight: 600;
}

.preview-texture-meta {
  color: var(--app-text-secondary);
  font-size: 11px;
}
</style>
