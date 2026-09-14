/**
 * WorkspacePreviewUtils - asset preview extraction helper.
 *
 * The class keeps Mesh and Texture2D preview extraction outside useWorkspace so the main
 * composable stays focused on shared page state instead of IPC and cache details.
 */

import { nextTick, type Ref } from 'vue'
import { invoke, Channel } from '@tauri-apps/api/core'
import { AssetClassificationUtils, LIKELY_EMPTY_MESH_PREVIEW_ERROR } from '../../utils/AssetClassificationUtils'
import { AssetDisplayUtils } from '../../utils/AssetDisplayUtils'
import { FormatUtils } from '../../utils/FormatUtils'
import type { AssetSummary, PreviewAnimationClipRef, PreviewAnimatorRef } from '../../types'
import { AssetMapCacheUtils } from '../../utils/AssetMapCacheUtils'
import type { AssetTypedPreviewResult } from '../../types'

/** Mesh geometry returned by the Rust extractor and consumed by Model3D. */
export interface MeshGeometryPreviewData {
  vertices: number[]
  indices: number[]
  normals?: number[]
  uvs?: number[]
  tangents?: number[]
  colors?: number[]
  sub_meshes?: MeshSubMeshPreviewData[]
  parts?: MeshGeometryPartPreviewData[]
  diffuse_texture_path?: string
  diffuse_texture_path_id?: string
  diffuse_texture_bundle_path?: string
  diffuse_texture_name?: string
  diffuse_material_name?: string
  diffuse_material_index?: number
  diffuse_slot_name?: string
  diffuse_texture_user_selected?: boolean
  selected_submesh_indices?: number[]
  texture_candidates?: MeshPreviewTextureCandidate[]
  glb_path?: string
  glb_url?: string
  glb_mesh_count?: number
  glb_material_count?: number
  glb_texture_count?: number
  glb_skeleton_joint_count?: number
  glb_animation_count?: number
  glb_animation_names?: string[]
  mesh_part_names?: string[]
  mesh_parts?: Array<{
    name: string
    bundle_path: string
    path_id: string
    class_name: string
  }>
  vertex_count?: number
  triangle_count?: number
  preview_request_key?: string
}

export interface AnimatorPreviewGlbResult {
  success: boolean
  error: string
  glb_path: string
  mesh_count: number
  material_count: number
  texture_count: number
  skeleton_joint_count: number
  animation_count: number
  animation_names: string[]
  mesh_part_names: string[]
  mesh_parts?: Array<{
    name: string
    bundle_path: string
    path_id: string
    class_name: string
  }>
  vertex_count: number
  triangle_count: number
  texture_candidates: MeshPreviewTextureCandidate[]
}

/** Incremental preview update sent phase-by-phase from the Rust backend. */
export interface IncrementalPreviewPayload {
  phase: string
  glb_path?: string
  vertex_count?: number
  triangle_count?: number
  mesh_count?: number
  material_count?: number
  texture_count?: number
  skeleton_joint_count?: number
  animation_count?: number
  animation_names?: string[]
  mesh_part_names?: string[]
  mesh_parts?: Array<{
    name: string
    bundle_path: string
    path_id: string
    class_name: string
  }>
  texture_candidates?: MeshPreviewTextureCandidate[]
}

export interface MeshGeometryPartPreviewData {
  name: string
  bundle_path: string
  path_id: string
  vertices: number[]
  indices: number[]
  normals?: number[]
  uvs?: number[]
  sub_meshes?: MeshSubMeshPreviewData[]
  vertex_count: number
  triangle_count: number
}

export interface MeshSubMeshPreviewData {
  index_start: number
  index_count: number
  topology: number
}

/** Texture preview metadata returned by the Rust texture preview command. */
export interface TexturePreviewCacheEntry {
  png_path: string
  png_data_url?: string
  preview_width?: number
  preview_height?: number
  width: number
  height: number
  texture_format: number
  texture_format_name: string
  complete_image_size: number
  mip_count: number
  image_count: number
  texture_dimension: number
  texture_dimension_name: string
  byte_size: number
  has_alpha: boolean
  color_space: number
  color_space_name: string
  filter_mode: number
  filter_mode_name: string
  aniso: number
  wrap_u: number
  wrap_v: number
  wrap_mode_name: string
  is_readable: boolean
  streaming_mipmaps: boolean
}

/** Mutable preview refs and caches owned by useWorkspace. */
export interface WorkspacePreviewState {
  assetProperties: Ref<Record<string, string>>
  previewFilePath: Ref<string>
  previewLoading: Ref<boolean>
  previewError: Ref<string>
  meshGeometryData: Ref<MeshGeometryPreviewData | null>
  textureInfo: Ref<TexturePreviewCacheEntry | null>
  typedPreviewData: Ref<AssetTypedPreviewResult | null>
  previewAnimators: Ref<PreviewAnimatorRef[]>
  previewAnimationClips: Ref<PreviewAnimationClipRef[]>
  meshCache: Map<string, MeshGeometryPreviewData | null>
  textureCache: Map<string, TexturePreviewCacheEntry | null>
  typedPreviewCache: Map<string, AssetTypedPreviewResult | null>
  previewAnimatorCache: Map<string, PreviewAnimatorRef[]>
  previewAnimationClipCache: Map<string, PreviewAnimationClipRef[]>
  selectedPreviewAnimationClip: Ref<PreviewAnimationClipRef | null>
  selectedPreviewMeshScope: Ref<{ bundle_path: string; path_id: string; class_name: string; name?: string } | null>
  textureByteSizeCache: Map<string, number>
  stopSignal: Ref<number>
}

export interface MeshPreviewTextureCandidate {
  png_path: string
  cache_png_path?: string
  texture_path_id: string
  texture_name: string
  texture_bundle_path: string
  material_name: string
  material_index: number
  slot_name: string
  usage: string
  width: number
  height: number
  texture_format: number
  texture_format_name: string
  byte_size: number
  has_alpha: boolean
  color_space_name: string
  wrap_mode_name: string
  filter_mode_name: string
  candidate_index?: number
}

export interface AssetPreviewOptions {
  models: boolean
  textures: boolean
  animations: boolean
  selected_animation_clip?: {
    bundle_path: string
    path_id: string
  } | null
}

export interface PreviewMeshScope {
  bundle_path: string
  path_id: string
  class_name: string
  name?: string
}

/** Preview helper methods for Mesh and Texture2D assets. */
export class WorkspacePreviewUtils {
  /** Keep insertion-ordered preview caches bounded without throwing away every tab switch. */
  static trimPreviewCache<K, V>(cache: Map<K, V>, limit: number): void {
    while (cache.size > limit) {
      const oldestKey = cache.keys().next().value
      if (oldestKey === undefined) return
      cache.delete(oldestKey)
    }
  }

  private static setPreviewCacheEntry<K, V>(cache: Map<K, V>, key: K, value: V, limit = 256): void {
    if (cache.has(key)) cache.delete(key)
    cache.set(key, value)
    WorkspacePreviewUtils.trimPreviewCache(cache, limit)
  }

  /** Format an asset byte size, preferring decoded texture byte size when available. */
  static getAssetSize(
    asset: { class_name: string; path_id: string; byte_size: number; source_bundle_path?: string },
    textureByteSizeCache: Map<string, number>,
    bundlePath?: string,
  ): string {
    if (asset.class_name === 'Texture2D' || asset.class_name === 'Sprite' || asset.class_name === 'SpriteMask') {
      const actualByteSize = textureByteSizeCache.get(
        WorkspacePreviewUtils.getTextureAssetCacheKey(asset.source_bundle_path || bundlePath || '', asset.path_id),
      )
      if (actualByteSize !== undefined) {
        return WorkspacePreviewUtils.formatByteSize(actualByteSize)
      }
    }
    return WorkspacePreviewUtils.formatByteSize(asset.byte_size)
  }

  /** Format bytes in KB/MB using the existing WorkSpace display convention. */
  private static formatByteSize(bytes: number): string {
    const kilobytes = bytes / 1024
    if (kilobytes >= 1024) {
      return (kilobytes / 1024).toFixed(1) + ' MB'
    }
    return kilobytes.toFixed(1) + ' KB'
  }

  /** Build basic properties that every asset displays before preview-specific metadata. */
  static buildBasicAssetProperties(asset: AssetSummary): Record<string, string> {
    return {
      Name: AssetDisplayUtils.getAssetDisplayName(asset),
      Path: asset.path,
      Type: asset.class_name,
      ClassID: String(asset.class_id),
      PathID: String(asset.path_id),
      DataSize: FormatUtils.formatSize(asset.byte_size),
    }
  }

  /** Extract or restore preview data for a selected asset. */
  static async loadAssetPreview(options: {
    asset: AssetSummary
    bundlePath: string
    cacheDir: string
    workspaceDirectory: string
    previewOptions?: AssetPreviewOptions
    state: WorkspacePreviewState
    t?: (key: string, named?: Record<string, unknown>) => string
    addLog: (message: string, type?: 'info' | 'success' | 'warn' | 'error') => void
  }): Promise<void> {
    const { asset, bundlePath, cacheDir, workspaceDirectory, state, addLog } = options
    const previewOptions = options.previewOptions || WorkspacePreviewUtils.defaultPreviewOptions()
    const t = options.t || ((key: string) => key)
    const properties = WorkspacePreviewUtils.buildBasicAssetProperties(asset)

    state.assetProperties.value = properties
    state.previewLoading.value = true

    if (asset.class_name === 'Mesh' || asset.class_name === 'Animator' || asset.class_name === 'GameObject') {
      await WorkspacePreviewUtils.loadMeshPreview(asset, bundlePath, cacheDir, workspaceDirectory, properties, previewOptions, state, t, addLog)
      return
    }

    if (asset.class_name === 'Texture2D' || asset.class_name === 'Sprite' || asset.class_name === 'SpriteMask') {
      await WorkspacePreviewUtils.loadTexturePreview(asset, bundlePath, cacheDir, state, t, addLog)
      return
    }

    await WorkspacePreviewUtils.loadTypedPreview(asset, bundlePath, workspaceDirectory, properties, state, addLog)
  }

  static defaultPreviewOptions(): AssetPreviewOptions {
    return {
      models: true,
      textures: false,
      animations: false,
      selected_animation_clip: null,
    }
  }

  static texturePreviewOptions(): AssetPreviewOptions {
    return {
      models: true,
      textures: true,
      animations: false,
      selected_animation_clip: null,
    }
  }

  static async scanPreviewAnimators(options: {
    asset: AssetSummary
    bundlePath: string
    workspaceDirectory: string
    assetMapCacheRoot: string
    cacheDir: string
    state: WorkspacePreviewState
    meshScope?: PreviewMeshScope | null
    addLog: (message: string, type?: 'info' | 'success' | 'warn' | 'error') => void
  }): Promise<void> {
    const { asset, bundlePath, workspaceDirectory, assetMapCacheRoot, cacheDir, state, addLog, meshScope } = options
    const scope = meshScope || state.selectedPreviewMeshScope.value
    const scanBundlePath = scope?.bundle_path || bundlePath
    const scanPathId = scope?.path_id || asset.path_id
    const scanClassName = scope?.class_name || asset.class_name
    const cacheKey = `${bundlePath}::${asset.path_id}::animators::${scanClassName}::${scanBundlePath}::${scanPathId}`
    const cached = state.previewAnimatorCache.get(cacheKey)
    if (cached) {
      state.previewAnimators.value = cached
      addLog(`  -> Animator cache hit: ${cached.length} animator(s)`, 'success')
      return
    }

    if (asset.class_name === 'Animator' || asset.class_name === 'Animation') {
      const current: PreviewAnimatorRef = {
        bundle_path: bundlePath,
        path_id: asset.path_id,
        class_name: asset.class_name,
        name: AssetDisplayUtils.getAssetDisplayName(asset),
        byte_size: asset.byte_size,
      }
      state.previewAnimators.value = [current]
      WorkspacePreviewUtils.setPreviewCacheEntry(state.previewAnimatorCache, cacheKey, [current])
      addLog(`  -> Current ${asset.class_name} is available for AnimationClip scan`, 'success')
      return
    }

    const progressChannel = new Channel<{ step: string; message: string }>()
    progressChannel.onmessage = (message) => addLog(`  -> ${message.message}`, 'info')
    addLog(`Scanning Animators for ${scanClassName} '${scope?.name || asset.path}'`, 'info')
    try {
      const animators = await invoke<PreviewAnimatorRef[]>('resolve_preview_animators', {
        bundlePath: scanBundlePath,
        pathId: scanPathId,
        className: scanClassName,
        workspaceDir: workspaceDirectory,
        assetMapCacheRoot,
        cacheDir,
        progress: progressChannel,
      })
      state.previewAnimators.value = animators
      WorkspacePreviewUtils.setPreviewCacheEntry(state.previewAnimatorCache, cacheKey, animators)
      addLog(`  -> Animator scan ready: ${animators.length} animator(s)`, animators.length ? 'success' : 'info')
    } catch (error) {
      addLog(`  -> Animator scan failed: ${error}`, 'warn')
    }
  }

  static async scanPreviewAnimations(options: {
    asset: AssetSummary
    bundlePath: string
    workspaceDirectory: string
    assetMapCacheRoot: string
    cacheDir: string
    state: WorkspacePreviewState
    meshScope?: PreviewMeshScope | null
    addLog: (message: string, type?: 'info' | 'success' | 'warn' | 'error') => void
  }): Promise<void> {
    const { asset, bundlePath, workspaceDirectory, assetMapCacheRoot, cacheDir, state, addLog, meshScope } = options
    if (!state.previewAnimators.value.length) {
      addLog('  -> Scan Animator first before scanning AnimationClips', 'warn')
      return
    }
    const scope = meshScope || state.selectedPreviewMeshScope.value
    const scanBundlePath = scope?.bundle_path || bundlePath
    const scanPathId = scope?.path_id || asset.path_id
    const scanClassName = scope?.class_name || asset.class_name
    const cacheKey = `${bundlePath}::${asset.path_id}::animation-clips::playable-v2::${scanClassName}::${scanBundlePath}::${scanPathId}`
    const cached = state.previewAnimationClipCache.get(cacheKey)
    if (cached) {
      state.previewAnimationClips.value = cached
      addLog(`  -> AnimationClip cache hit: ${cached.length} clip(s)`, 'success')
      return
    }

    const progressChannel = new Channel<{ step: string; message: string }>()
    progressChannel.onmessage = (message) => addLog(`  -> ${message.message}`, 'info')
    addLog(`Scanning AnimationClips for ${scanClassName} '${scope?.name || asset.path}'`, 'info')
    try {
      const clips = await invoke<PreviewAnimationClipRef[]>('resolve_preview_animation_clips', {
        bundlePath: scanBundlePath,
        pathId: scanPathId,
        className: scanClassName,
        workspaceDir: workspaceDirectory,
        assetMapCacheRoot,
        cacheDir,
        progress: progressChannel,
      })
      state.previewAnimationClips.value = clips
      WorkspacePreviewUtils.setPreviewCacheEntry(state.previewAnimationClipCache, cacheKey, clips)
      addLog(`  -> AnimationClip scan ready: ${clips.length} clip(s)`, clips.length ? 'success' : 'info')
    } catch (error) {
      addLog(`  -> AnimationClip scan failed: ${error}`, 'warn')
    }
  }

  static async applyPreviewAnimationClip(options: {
    clip: PreviewAnimationClipRef
    asset: AssetSummary
    bundlePath: string
    cacheDir: string
    workspaceDirectory: string
    state: WorkspacePreviewState
    t?: (key: string, named?: Record<string, unknown>) => string
    addLog: (message: string, type?: 'info' | 'success' | 'warn' | 'error') => void
  }): Promise<void> {
    const { clip, asset, bundlePath, cacheDir, workspaceDirectory, state, addLog } = options
    const t = options.t || ((key: string) => key)
    const properties = WorkspacePreviewUtils.buildBasicAssetProperties(asset)
    state.selectedPreviewAnimationClip.value = clip
    state.previewError.value = ''
    await WorkspacePreviewUtils.loadMeshPreview(
      asset,
      bundlePath,
      cacheDir,
      workspaceDirectory,
      properties,
      {
        models: true,
        textures: Boolean(state.meshGeometryData.value?.texture_candidates?.length || state.meshGeometryData.value?.glb_texture_count),
        animations: true,
        selected_animation_clip: {
          bundle_path: clip.bundle_path,
          path_id: clip.path_id,
        },
      },
      state,
      t,
      addLog,
    )
  }

  static async scanPreviewTextures(options: {
    asset: AssetSummary
    bundlePath: string
    cacheDir: string
    workspaceDirectory: string
    state: WorkspacePreviewState
    t?: (key: string, named?: Record<string, unknown>) => string
    addLog: (message: string, type?: 'info' | 'success' | 'warn' | 'error') => void
  }): Promise<void> {
    const { asset, bundlePath, cacheDir, workspaceDirectory, state, addLog } = options
    const t = options.t || ((key: string) => key)
    const properties = WorkspacePreviewUtils.buildBasicAssetProperties(asset)
    state.previewError.value = ''

    try {
      if (asset.class_name === 'Mesh' && state.meshGeometryData.value) {
        await WorkspacePreviewUtils.tryApplyDiffuseTexturePreview(
          state.meshGeometryData.value as MeshGeometryPreviewData & { vertex_count: number; triangle_count: number },
          asset,
          bundlePath,
          workspaceDirectory,
          cacheDir,
          state,
          t,
          addLog,
        )
        return
      }

      await WorkspacePreviewUtils.loadMeshPreview(
        asset,
        bundlePath,
        cacheDir,
        workspaceDirectory,
        properties,
        WorkspacePreviewUtils.texturePreviewOptions(),
        state,
        t,
        addLog,
      )
    } finally {
      state.previewLoading.value = false
    }
  }

  /** Load Mesh geometry for direct Three.js rendering, with a per-bundle cache. */
  private static async loadMeshPreview(
    asset: AssetSummary,
    bundlePath: string,
    cacheDir: string,
    workspaceDirectory: string,
    properties: Record<string, string>,
    previewOptions: AssetPreviewOptions,
    state: WorkspacePreviewState,
    t: (key: string, named?: Record<string, unknown>) => string,
    addLog: (message: string, type?: 'info' | 'success' | 'warn' | 'error') => void,
  ): Promise<void> {
    if (!previewOptions.models) {
      state.assetProperties.value = properties
      state.meshGeometryData.value = null
      state.previewFilePath.value = ''
      state.previewError.value = 'Model preview disabled'
      state.previewLoading.value = false
      addLog('  -> Model preview disabled by selection', 'info')
      return
    }

    const cacheKey = WorkspacePreviewUtils.getMeshPreviewCacheKey(
      bundlePath,
      asset.path_id,
      asset.class_name === 'Mesh' ? undefined : previewOptions,
    )

    if (AssetClassificationUtils.isLikelyEmptyMesh(asset)) {
      state.assetProperties.value = {
        ...properties,
        Vertices: '0',
        Triangles: '0',
        HasIndices: 'No',
        RawDataSize: FormatUtils.formatSize(asset.byte_size),
      }
      state.meshCache.delete(cacheKey)
      state.previewError.value = LIKELY_EMPTY_MESH_PREVIEW_ERROR
      state.meshGeometryData.value = null
      state.previewLoading.value = false
      addLog(`  -> ${t('previewLog.skippedLikelyEmptyMesh', { size: asset.byte_size })}`, 'info')
      return
    }

    const cached = state.meshCache.get(cacheKey)

    if (cached !== null && cached !== undefined) {
      const restored = {
        ...cached,
        preview_request_key: `${bundlePath}::${asset.path_id}`,
      }
      WorkspacePreviewUtils.applyMeshData(asset, properties, restored, state)
      state.previewLoading.value = false
      const textureCount = restored.texture_candidates?.length || 0
      addLog(`  -> ${t(
        textureCount ? 'previewLog.meshPreviewCacheHitWithTextures' : 'previewLog.meshPreviewCacheHit',
        { vertices: Math.floor(restored.vertices.length / 3), textures: textureCount },
      )}`, 'success')
      if (previewOptions.textures && asset.class_name === 'Mesh' && textureCount === 0) {
        await WorkspacePreviewUtils.tryApplyDiffuseTexturePreview(
          restored as MeshGeometryPreviewData & { vertex_count: number; triangle_count: number },
          asset,
          bundlePath,
          workspaceDirectory,
          cacheDir,
          state,
          t,
          addLog,
        )
      }
      return
    }

    if (cached === null) {
      state.meshCache.delete(cacheKey)
      addLog(`  -> ${t('previewLog.retryStaleMeshPreviewCache')}`, 'info')
    }

    addLog(t('previewLog.extractingGeometry', { name: asset.path }), 'info')
    const progressChannel = new Channel<{ step: string; message: string }>()
    progressChannel.onmessage = (message) => addLog(`  -> ${message.message}`, 'info')

    const previewRequestKey = `${bundlePath}::${asset.path_id}`
    let incrementalGeometry: MeshGeometryPreviewData & {
      success: boolean
      error: string
      vertex_count: number
      triangle_count: number
    } = {
      vertices: [],
      indices: [],
      normals: [],
      uvs: [],
      success: true,
      error: '',
      vertex_count: 0,
      triangle_count: 0,
      preview_request_key: previewRequestKey,
    }

    let geometry!: MeshGeometryPreviewData & {
      success: boolean
      error: string
      vertex_count: number
      triangle_count: number
    }

    // Incremental preview: receive phase-by-phase updates from the Rust backend.
    const incrementalChannel = new Channel<IncrementalPreviewPayload>()
    incrementalChannel.onmessage = (inc) => {
      addLog(`  -> [${inc.phase}] mesh=${inc.mesh_count ?? '?'} verts=${inc.vertex_count ?? '?'} tris=${inc.triangle_count ?? '?'} textures=${inc.texture_count ?? '?'} glb=${inc.glb_path ? 'yes' : 'no'}`, 'info')
      incrementalGeometry = {
        ...incrementalGeometry,
        ...(inc.glb_path ? { glb_path: inc.glb_path } : {}),
        ...(inc.mesh_count != null ? { glb_mesh_count: inc.mesh_count } : {}),
        ...(inc.vertex_count != null ? { vertex_count: inc.vertex_count } : {}),
        ...(inc.triangle_count != null ? { triangle_count: inc.triangle_count } : {}),
        ...(inc.material_count != null ? { glb_material_count: inc.material_count } : {}),
        ...(inc.texture_count != null ? { glb_texture_count: inc.texture_count } : {}),
        ...(inc.skeleton_joint_count != null ? { glb_skeleton_joint_count: inc.skeleton_joint_count } : {}),
        ...(inc.animation_count != null ? { glb_animation_count: inc.animation_count } : {}),
        ...(inc.animation_names ? { glb_animation_names: inc.animation_names } : {}),
        ...(inc.mesh_part_names ? { mesh_part_names: inc.mesh_part_names } : {}),
        ...(inc.mesh_parts ? { mesh_parts: inc.mesh_parts } : {}),
        ...(inc.texture_candidates ? { texture_candidates: inc.texture_candidates } : {}),
      }
      // Hide loading spinner immediately so the user sees data as it arrives.
      state.previewLoading.value = false
      // Spread into a new object so Vue reactivity detects the change (same-ref assignment is skipped).
      WorkspacePreviewUtils.applyMeshData(
        asset,
        properties,
        { ...incrementalGeometry },
        state,
        incrementalGeometry.vertex_count,
        incrementalGeometry.triangle_count,
      )
    }

    await nextTick()

    try {
      geometry = asset.class_name === 'Animator'
        ? await WorkspacePreviewUtils.invokeAnimatorGeometry(asset, bundlePath, cacheDir, workspaceDirectory, previewOptions, progressChannel)
        : asset.class_name === 'GameObject'
          ? await WorkspacePreviewUtils.invokeGameObjectGeometry(asset, bundlePath, cacheDir, workspaceDirectory, previewOptions, progressChannel, incrementalChannel)
        : await invoke<MeshGeometryPreviewData & {
        success: boolean
        error: string
        vertex_count: number
        triangle_count: number
      }>('extract_mesh_geometry', {
        bundlePath,
        pathId: asset.path_id,
        progress: progressChannel,
        cacheDir,
      })

      if (geometry.success) {
        geometry.preview_request_key = previewRequestKey
        if (asset.class_name === 'GameObject' && incrementalGeometry.glb_path) {
          geometry = {
            ...incrementalGeometry,
            ...geometry,
            glb_path: incrementalGeometry.glb_path,
            glb_mesh_count: incrementalGeometry.glb_mesh_count ?? geometry.glb_mesh_count,
            glb_material_count: incrementalGeometry.glb_material_count ?? geometry.glb_material_count,
            glb_texture_count: incrementalGeometry.glb_texture_count ?? geometry.glb_texture_count,
            glb_skeleton_joint_count: incrementalGeometry.glb_skeleton_joint_count ?? geometry.glb_skeleton_joint_count,
            glb_animation_count: incrementalGeometry.glb_animation_count ?? geometry.glb_animation_count,
            glb_animation_names: incrementalGeometry.glb_animation_names ?? geometry.glb_animation_names,
            mesh_part_names: incrementalGeometry.mesh_part_names ?? geometry.mesh_part_names,
            mesh_parts: incrementalGeometry.mesh_parts ?? geometry.mesh_parts,
            texture_candidates: incrementalGeometry.texture_candidates ?? geometry.texture_candidates,
            diffuse_texture_path: incrementalGeometry.diffuse_texture_path ?? geometry.diffuse_texture_path,
            diffuse_texture_path_id: incrementalGeometry.diffuse_texture_path_id ?? geometry.diffuse_texture_path_id,
            diffuse_texture_bundle_path: incrementalGeometry.diffuse_texture_bundle_path ?? geometry.diffuse_texture_bundle_path,
            diffuse_texture_name: incrementalGeometry.diffuse_texture_name ?? geometry.diffuse_texture_name,
            diffuse_material_name: incrementalGeometry.diffuse_material_name ?? geometry.diffuse_material_name,
            diffuse_material_index: incrementalGeometry.diffuse_material_index ?? geometry.diffuse_material_index,
            diffuse_slot_name: incrementalGeometry.diffuse_slot_name ?? geometry.diffuse_slot_name,
          }
        }
        WorkspacePreviewUtils.applyMeshData(asset, properties, geometry, state, geometry.vertex_count, geometry.triangle_count)
        addLog(`  -> ${t('previewLog.verticesTriangles', {
          vertices: geometry.vertex_count || Math.floor(geometry.vertices.length / 3),
          triangles: geometry.triangle_count || Math.floor(geometry.indices.length / 3),
        })}`, 'success')
        if (previewOptions.textures && asset.class_name === 'Mesh') {
          await WorkspacePreviewUtils.tryApplyDiffuseTexturePreview(
            geometry,
            asset,
            bundlePath,
            workspaceDirectory,
            cacheDir,
            state,
            t,
            addLog,
          )
        }
        WorkspacePreviewUtils.setPreviewCacheEntry(state.meshCache, cacheKey, {
          ...(state.meshGeometryData.value || geometry),
          preview_request_key: undefined,
        })
      } else {
        state.meshCache.delete(cacheKey)
        const error = geometry.error || 'Cannot extract Mesh geometry data'
        if (WorkspacePreviewUtils.isLikelyEmptyMeshError(error)) {
          state.assetProperties.value = {
            ...properties,
            Vertices: '0',
            Triangles: '0',
            HasIndices: 'No',
            RawDataSize: FormatUtils.formatSize(asset.byte_size),
          }
          state.previewError.value = LIKELY_EMPTY_MESH_PREVIEW_ERROR
        } else {
          state.assetProperties.value = { ...properties, Vertices: '?', Triangles: '?', HasIndices: '?' }
          state.previewError.value = error
        }
        state.meshGeometryData.value = null
        addLog(`  -> ${t('previewLog.extractionFailed', { error: geometry.error })}`, 'warn')
      }
    } catch (error) {
      state.meshCache.delete(cacheKey)
      state.previewError.value = String(error)
      state.meshGeometryData.value = null
      addLog(`  ${t('previewLog.extractionFailed', { error })}`, 'error')
    } finally {
      state.previewLoading.value = false
    }
  }

  static getMeshPreviewCacheKey(
    bundlePath: string,
    pathId: string,
    previewOptions?: AssetPreviewOptions,
  ): string {
    const optionSuffix = previewOptions
      ? `::models=${previewOptions.models ? '1' : '0'}::textures=${previewOptions.textures ? '1' : '0'}::animations=${previewOptions.animations ? '1' : '0'}::clip=${previewOptions.selected_animation_clip ? `${previewOptions.selected_animation_clip.bundle_path}:${previewOptions.selected_animation_clip.path_id}` : ''}`
      : ''
    return `${bundlePath}::${pathId}::preview-v4${optionSuffix}`
  }

  private static async invokeAnimatorGeometry(
    asset: AssetSummary,
    bundlePath: string,
    cacheDir: string,
    workspaceDirectory: string,
    previewOptions: AssetPreviewOptions,
    progressChannel: Channel<{ step: string; message: string }>,
  ): Promise<MeshGeometryPreviewData & {
    success: boolean
    error: string
    vertex_count: number
    triangle_count: number
  }> {
    const assetMapCacheRoot = await AssetMapCacheUtils.getCacheRoot()
    const result = await invoke<AnimatorPreviewGlbResult>('extract_animator_preview_glb', {
      bundlePath,
      pathId: asset.path_id,
      workspaceDir: workspaceDirectory,
      cacheDir,
      assetMapCacheRoot,
      options: previewOptions,
      progress: progressChannel,
    })
    return WorkspacePreviewUtils.glbResultToMeshGeometry(result)
  }

  private static async invokeGameObjectGeometry(
    asset: AssetSummary,
    bundlePath: string,
    cacheDir: string,
    workspaceDirectory: string,
    previewOptions: AssetPreviewOptions,
    progressChannel: Channel<{ step: string; message: string }>,
    incrementalChannel?: Channel<IncrementalPreviewPayload>,
  ): Promise<MeshGeometryPreviewData & {
    success: boolean
    error: string
    vertex_count: number
    triangle_count: number
  }> {
    const assetMapCacheRoot = await AssetMapCacheUtils.getCacheRoot()
    const result = await invoke<AnimatorPreviewGlbResult>('extract_game_object_preview_glb', {
      bundlePath,
      pathId: asset.path_id,
      workspaceDir: workspaceDirectory,
      cacheDir,
      assetMapCacheRoot,
      options: previewOptions,
      progress: progressChannel,
      incremental: incrementalChannel,
    })
    return WorkspacePreviewUtils.glbResultToMeshGeometry(result)
  }

  private static glbResultToMeshGeometry(result: AnimatorPreviewGlbResult): MeshGeometryPreviewData & {
    success: boolean
    error: string
    vertex_count: number
    triangle_count: number
  } {
    return {
      vertices: [],
      indices: [],
      normals: [],
      uvs: [],
      success: result.success,
      error: result.error,
      vertex_count: result.vertex_count,
      triangle_count: result.triangle_count,
      glb_path: result.glb_path,
      glb_mesh_count: result.mesh_count,
      glb_material_count: result.material_count,
      glb_texture_count: result.texture_count,
      glb_skeleton_joint_count: result.skeleton_joint_count,
      glb_animation_count: result.animation_count,
      glb_animation_names: result.animation_names,
      mesh_part_names: result.mesh_part_names || [],
      mesh_parts: result.mesh_parts || [],
      texture_candidates: result.texture_candidates,
      diffuse_texture_name: result.texture_count > 0 ? `${result.texture_count} texture(s)` : '',
      diffuse_material_name: result.material_count > 0 ? `${result.material_count} material(s)` : '',
      diffuse_slot_name: '',
    }
  }

  static isLikelyEmptyMeshError(error: string): boolean {
    if (error.includes('\u7a7a Mesh')) return true
    return error.includes('疑似Empty Mesh') || error.toLowerCase().includes('likely empty mesh')
  }

  static getTextureAssetCacheKey(bundlePath: string, pathId: string): string {
    return `${bundlePath}::${pathId}`
  }

  static getTexturePreviewCacheKey(bundlePath: string, pathId: string, maxPreviewEdge: number): string {
    return `${WorkspacePreviewUtils.getTextureAssetCacheKey(bundlePath, pathId)}::edge=${maxPreviewEdge}`
  }

  private static async tryApplyDiffuseTexturePreview(
    geometry: MeshGeometryPreviewData & { vertex_count: number; triangle_count: number },
    asset: AssetSummary,
    bundlePath: string,
    workspaceDirectory: string,
    cacheDir: string,
    state: WorkspacePreviewState,
    t: (key: string, named?: Record<string, unknown>) => string,
    addLog: (message: string, type?: 'info' | 'success' | 'warn' | 'error') => void,
  ): Promise<void> {
    if (!workspaceDirectory) return
    const vertexCount = geometry.vertex_count || Math.floor(geometry.vertices.length / 3)
    if (!geometry.uvs || geometry.uvs.length < vertexCount * 2) {
      addLog(`  -> ${t('previewLog.diffuseNoUv')}`, 'info')
      return
    }

    try {
      const progressChannel = new Channel<{ step: string; message: string }>()
      progressChannel.onmessage = (message) => addLog(`  -> ${message.message}`, 'info')
      const previewRequestKey = `${bundlePath}::${asset.path_id}`
      const meshCacheKey = WorkspacePreviewUtils.getMeshPreviewCacheKey(
        bundlePath,
        asset.path_id,
      )
      const assetMapCacheRoot = await AssetMapCacheUtils.getCacheRoot()
      const stopVersion = state.stopSignal.value
      const candidates = await invoke<MeshPreviewTextureCandidate[]>('resolve_mesh_preview_texture_candidates', {
        bundlePath,
        pathId: asset.path_id,
        workspaceDir: workspaceDirectory,
        cacheDir,
        assetMapCacheRoot,
        progress: progressChannel,
      })
      if (!candidates.length) {
        addLog(`  -> ${t('previewLog.diffuseNoCandidates')}`, 'info')
        return
      }
      addLog(`  -> ${t('previewLog.textureCandidatesFound', { count: candidates.length })}`, 'info')

      const liveGeometry = state.meshGeometryData.value
      if (!liveGeometry || liveGeometry.preview_request_key !== previewRequestKey) return
      liveGeometry.texture_candidates = []
      state.meshGeometryData.value = { ...liveGeometry }

      for (let index = 0; index < candidates.length; index++) {
        if (state.stopSignal.value !== stopVersion) {
          addLog(`  -> ${t('previewLog.textureThumbnailStopped')}`, 'warn')
          return
        }
        const candidate = candidates[index]
        const stillCurrent = state.meshGeometryData.value
        if (!stillCurrent || stillCurrent.preview_request_key !== previewRequestKey) return

        try {
          const textureCacheKey = WorkspacePreviewUtils.getTexturePreviewCacheKey(
            candidate.texture_bundle_path,
            candidate.texture_path_id,
            512,
          )
          let preview = state.textureCache.get(textureCacheKey)
          if (preview === undefined) {
            preview = await invoke<TexturePreviewCacheEntry>('export_texture_preview', {
              bundlePath: candidate.texture_bundle_path,
              pathId: candidate.texture_path_id,
              cacheDir,
              maxPreviewEdge: 512,
            })
            WorkspacePreviewUtils.setPreviewCacheEntry(state.textureCache, textureCacheKey, preview)
            state.textureByteSizeCache.set(
              WorkspacePreviewUtils.getTextureAssetCacheKey(candidate.texture_bundle_path, candidate.texture_path_id),
              preview.byte_size,
            )
          } else if (preview === null) {
            throw new Error(t('previewLog.texturePreviewFailedCached'))
          }
          if (state.stopSignal.value !== stopVersion) {
            addLog(`  -> ${t('previewLog.textureThumbnailStopped')}`, 'warn')
            return
          }
          const hydratedCandidate: MeshPreviewTextureCandidate = {
            ...candidate,
            png_path: preview.png_data_url || preview.png_path,
            cache_png_path: preview.png_path,
            width: preview.width,
            height: preview.height,
            texture_format: preview.texture_format,
            texture_format_name: preview.texture_format_name,
            byte_size: preview.byte_size,
            has_alpha: preview.has_alpha,
            color_space_name: preview.color_space_name,
            wrap_mode_name: preview.wrap_mode_name,
            filter_mode_name: preview.filter_mode_name,
          }

          const currentGeometry: MeshGeometryPreviewData | null = state.meshGeometryData.value
          if (!currentGeometry || currentGeometry.preview_request_key !== previewRequestKey) return
          const updatedGeometry: MeshGeometryPreviewData = {
            ...currentGeometry,
            texture_candidates: [
            ...(currentGeometry.texture_candidates || []),
            hydratedCandidate,
            ],
          }
          state.meshGeometryData.value = updatedGeometry
          WorkspacePreviewUtils.setPreviewCacheEntry(state.meshCache, meshCacheKey, {
            ...updatedGeometry,
            preview_request_key: undefined,
          })
          addLog(`  -> ${t('previewLog.textureThumbnailLoaded', {
            name: hydratedCandidate.texture_name,
            index: index + 1,
            total: candidates.length,
          })}`, 'success')
          await nextTick()
        } catch (textureError) {
          addLog(`  -> ${t('previewLog.textureThumbnailSkipped', {
            name: candidate.texture_name,
            error: textureError,
          })}`, 'warn')
        }
      }
    } catch (error) {
      addLog(`  -> ${t('previewLog.diffuseFailed', { error })}`, 'warn')
    }
  }

  /** Apply successful Mesh preview data to all dependent refs. */
  private static applyMeshData(
    asset: AssetSummary,
    properties: Record<string, string>,
    geometry: MeshGeometryPreviewData,
    state: WorkspacePreviewState,
    vertexCount?: number,
    triangleCount?: number,
  ): void {
    state.assetProperties.value = {
      ...properties,
      Vertices: String(vertexCount || Math.floor(geometry.vertices.length / 3) || '?'),
      Triangles: String(triangleCount || Math.floor(geometry.indices.length / 3) || '?'),
      HasIndices: geometry.indices.length > 0 || geometry.glb_path ? 'Yes' : 'No',
      Normals: geometry.normals && geometry.normals.length >= (vertexCount || Math.floor(geometry.vertices.length / 3)) * 3 ? 'Yes' : 'No',
      UV0: geometry.uvs && geometry.uvs.length >= (vertexCount || Math.floor(geometry.vertices.length / 3)) * 2 ? 'Yes' : 'No',
      ...(geometry.parts?.length ? {
        MeshParts: String(geometry.parts.length),
      } : {}),
      ...(geometry.glb_path || geometry.glb_mesh_count ? {
        ...(geometry.glb_path ? { Preview: 'GLB' } : {}),
        MeshParts: String(geometry.glb_mesh_count || geometry.parts?.length || 1),
        Materials: String(geometry.glb_material_count || 0),
        Textures: String(geometry.glb_texture_count || 0),
        SkeletonJoints: String(geometry.glb_skeleton_joint_count || 0),
        Animations: String(geometry.glb_animation_count || 0),
      } : {}),
      ...(geometry.diffuse_texture_name ? {
        DiffuseTexture: geometry.diffuse_texture_name,
        DiffuseSlot: geometry.diffuse_slot_name || '?',
      } : {}),
      RawDataSize: FormatUtils.formatSize(asset.byte_size),
    }
    state.meshGeometryData.value = geometry
    state.previewFilePath.value = 'direct'
  }

  /** Load Texture2D/Sprite PNG preview with a per-bundle texture cache. */
  private static async loadTexturePreview(
    asset: AssetSummary,
    bundlePath: string,
    cacheDir: string,
    state: WorkspacePreviewState,
    t: (key: string, named?: Record<string, unknown>) => string,
    addLog: (message: string, type?: 'info' | 'success' | 'warn' | 'error') => void,
  ): Promise<void> {
    try {
      addLog(t('previewLog.exportingPreview', { name: asset.path, className: asset.class_name }), 'info')

      if (asset.class_name !== 'Texture2D' && asset.class_name !== 'Sprite' && asset.class_name !== 'SpriteMask') {
        return
      }

      const cacheKey = WorkspacePreviewUtils.getTexturePreviewCacheKey(bundlePath, asset.path_id, 2048)
      const cached = state.textureCache.get(cacheKey)

      if (cached !== null && cached !== undefined) {
        state.previewFilePath.value = cached.png_data_url || cached.png_path
        state.textureInfo.value = cached
        addLog(`  -> ${t('previewLog.textureCacheHit', {
          width: cached.width,
          height: cached.height,
          format: cached.texture_format_name,
        })}`, 'success')
        return
      }

      if (cached === null) {
        state.previewError.value = t('assetPreview.textureExportFailedCached')
        return
      }

      const result = await invoke<TexturePreviewCacheEntry>('export_texture_preview', {
        bundlePath,
        pathId: asset.path_id,
        cacheDir,
        maxPreviewEdge: 2048,
      })

      state.previewFilePath.value = result.png_data_url || result.png_path
      state.textureInfo.value = result
      WorkspacePreviewUtils.setPreviewCacheEntry(state.textureCache, cacheKey, result)
      state.textureByteSizeCache.set(
        WorkspacePreviewUtils.getTextureAssetCacheKey(bundlePath, asset.path_id),
        result.byte_size,
      )
      addLog(`  -> ${t('previewLog.texturePreviewReady', {
        width: result.width,
        height: result.height,
        format: result.texture_format_name,
      })}`, 'success')
    } catch (error) {
      WorkspacePreviewUtils.setPreviewCacheEntry(
        state.textureCache,
        WorkspacePreviewUtils.getTexturePreviewCacheKey(bundlePath, asset.path_id, 2048),
        null,
      )
      state.textureInfo.value = null
      state.previewError.value = String(error)
      addLog(`  -> ${t('previewLog.textureExportFailed', { error })}`, 'error')
    } finally {
      state.previewLoading.value = false
    }
  }

  private static async loadTypedPreview(
    asset: AssetSummary,
    bundlePath: string,
    workspaceDirectory: string,
    properties: Record<string, string>,
    state: WorkspacePreviewState,
    addLog: (message: string, type?: 'info' | 'success' | 'warn' | 'error') => void,
  ): Promise<void> {
    const cacheKey = `${bundlePath}::${asset.path_id}::typed`
    const cached = state.typedPreviewCache.get(cacheKey)
    if (cached !== undefined) {
      state.typedPreviewData.value = cached
      state.assetProperties.value = cached
        ? WorkspacePreviewUtils.propertiesFromTypedPreview(properties, cached)
        : properties
      state.previewLoading.value = false
      if (cached) addLog(`  -> Typed preview cache hit: ${cached.sections.length} section(s)`, 'success')
      return
    }

    try {
      addLog(`Loading typed preview for ${asset.class_name} '${asset.path}'`, 'info')
      const progressChannel = new Channel<{ step: string; message: string }>()
      progressChannel.onmessage = (message) => addLog(`  -> ${message.message}`, 'info')
      const assetMapCacheRoot = await AssetMapCacheUtils.getCacheRoot()
      const preview = await invoke<AssetTypedPreviewResult>('get_asset_typed_preview', {
        bundlePath,
        pathId: asset.path_id,
        workspaceDir: workspaceDirectory || undefined,
        assetMapCacheRoot,
        progress: progressChannel,
      })
      state.typedPreviewData.value = preview
      state.assetProperties.value = WorkspacePreviewUtils.propertiesFromTypedPreview(properties, preview)
      WorkspacePreviewUtils.setPreviewCacheEntry(state.typedPreviewCache, cacheKey, preview)
      addLog(`  -> Typed preview ready: ${preview.sections.length} section(s), ${preview.relations.length} relation(s)`, 'success')
    } catch (error) {
      state.typedPreviewData.value = null
      state.previewError.value = String(error)
      WorkspacePreviewUtils.setPreviewCacheEntry(state.typedPreviewCache, cacheKey, null)
      addLog(`  -> Typed preview failed: ${error}`, 'warn')
    } finally {
      state.previewLoading.value = false
    }
  }

  private static propertiesFromTypedPreview(
    base: Record<string, string>,
    preview: AssetTypedPreviewResult,
  ): Record<string, string> {
    const summary = preview.sections.find(section => section.title === 'Summary') || preview.sections[0]
    const extra = Object.fromEntries((summary?.rows || []).slice(0, 12).map(row => [row.label, row.value]))
    return {
      ...base,
      ...extra,
      Relations: String(preview.relations.length),
      Warnings: String(preview.warnings.length),
    }
  }
}
