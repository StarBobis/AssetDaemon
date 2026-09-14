<script setup lang="ts">
/**
 * Model3D - Unified Three.js 3D model preview component.
 *
 * Receives vertex/index data extracted by Rust, builds BufferGeometry directly in the browser.
 * No dependency on exported model files, completely avoids format conversion issues.
 *
 * Usage:
 *   <Model3D :geometry-data="{ vertices: [...], indices: [...] }" />
 */

import { ref, onMounted, onUnmounted, watch, computed } from 'vue'
import * as THREE from 'three'
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js'
import { GLTFLoader, type GLTF } from 'three/examples/jsm/loaders/GLTFLoader.js'
import { EffectComposer } from 'three/examples/jsm/postprocessing/EffectComposer.js'
import { OutlinePass } from 'three/examples/jsm/postprocessing/OutlinePass.js'
import { OutputPass } from 'three/examples/jsm/postprocessing/OutputPass.js'
import { RenderPass } from 'three/examples/jsm/postprocessing/RenderPass.js'
import { Refresh } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'

const { t } = useI18n()

const MAX_PREVIEW_VERTICES = 300_000
const MAX_PREVIEW_INDICES = 900_000
const MAX_COMPUTED_NORMAL_INDICES = 180_000
const PREVIEW_DEFAULT_COLORS = {
  meshFallback: 0x018b8d,
  outline: 0xffe15a,
  gridMajor: 0x666666,
  gridMinor: 0x444444,
  hemisphereSky: 0xdfeaff,
  hemisphereGround: 0x2d3340,
  keyLight: 0xfff0d8,
  fillLight: 0x9fbdff,
  rimLight: 0xd9ecff,
  backgroundFallback: 0x3d4652,
} as const
const SUBMESH_OUTLINE_COLOR = PREVIEW_DEFAULT_COLORS.outline

type PreviewTextureCandidate = {
  png_path: string
  cache_png_path?: string
  material_index: number
  material_name?: string
  texture_path_id?: string
  texture_bundle_path?: string
  texture_name?: string
  usage?: string
  slot_name?: string
  candidate_index?: number
  width?: number
  height?: number
  texture_format_name?: string
  color_space_name?: string
  wrap_mode_name?: string
  filter_mode_name?: string
}

type PreviewMaterialData = {
  diffuse_texture_path?: string
  diffuse_texture_path_id?: string
  diffuse_texture_bundle_path?: string
  diffuse_texture_name?: string
  diffuse_material_name?: string
  diffuse_material_index?: number
  diffuse_slot_name?: string
  diffuse_texture_user_selected?: boolean
  selected_submesh_indices?: number[]
  sub_meshes?: Array<{
    index_start: number
    index_count: number
    topology?: number
  }>
  texture_candidates?: PreviewTextureCandidate[]
}

type SubMeshControl = {
  key: string
  index: number
  name: string
  displayName: string
  visible: boolean
  selected: boolean
  color: string
}

type PartControl = {
  key: string
  index: number
  name: string
  displayName: string
  color: string
  visible: boolean
  selected: boolean
  bundlePath?: string
  pathId?: string
  className?: string
}

type MeshRegionsChangePayload = {
  subMeshes: SubMeshControl[]
  parts: PartControl[]
}

type MeshPreviewPart = {
  name?: string
  asset_name?: string
  bundle_path: string
  path_id: string
}

const props = defineProps<{
  /** Geometry data extracted from Rust side */
  geometryData: {
    vertices: number[]
    indices: number[]
    normals?: number[]
    uvs?: number[]
    sub_meshes?: Array<{
      index_start: number
      index_count: number
      topology: number
    }>
    selected_submesh_indices?: number[]
    parts?: Array<{
    name?: string
    asset_name?: string
    bundle_path: string
    path_id: string
    vertices: number[]
      indices: number[]
      normals?: number[]
      uvs?: number[]
      sub_meshes?: Array<{
        index_start: number
        index_count: number
        topology: number
      }>
      vertex_count: number
      triangle_count: number
    }>
    diffuse_texture_path?: string
    diffuse_texture_path_id?: string
    diffuse_texture_bundle_path?: string
    diffuse_texture_name?: string
    diffuse_material_name?: string
    diffuse_material_index?: number
    diffuse_slot_name?: string
    diffuse_texture_user_selected?: boolean
    texture_candidates?: PreviewTextureCandidate[]
    glb_url?: string
    glb_path?: string
    mesh_part_names?: string[]
    mesh_parts?: Array<{
      name: string
      bundle_path: string
      path_id: string
      class_name: string
    }>
    glb_animation_names?: string[]
    glb_animation_count?: number
  } | null
  /** Asset name (for display) */
  assetName: string
  /** Selected AnimationClip name, used to pick the matching GLB clip after reload. */
  preferredAnimationName?: string
}>()

const emit = defineEmits<{
  /** Emitted when geometry data indices are out of bounds, parent can show in log */
  (e: 'geometry-error', detail: { maxIdx: number; vertexCount: number; indexCount: number }): void
  /** Emitted when mesh region controls are rebuilt or selected by viewer interaction. */
  (e: 'mesh-regions-change', detail: MeshRegionsChangePayload): void
  /** Emitted when the active GLB animation starts, pauses, or is cleared. */
  (e: 'animation-playback-change', detail: {
    playing: boolean
    clipIndex: number | null
    clipName: string | null
    targetedBoneNames?: string[]
  }): void
}>()

// ============================================================
// State
// ============================================================

/** Container DOM reference */
const containerRef = ref<HTMLDivElement | null>(null)

/** Loading state */
const loadState = ref<'loading' | 'ready' | 'error'>('loading')
const loadError = ref('')
const partControls = ref<PartControl[]>([])
const subMeshControls = ref<SubMeshControl[]>([])
const animationClips = ref<Array<{ index: number; name: string; duration: number }>>([])
const selectedAnimationIndex = ref(0)
const isAnimationPlaying = ref(false)
const lastAppliedTextureCandidateKey = ref('')
const hasSubMeshControls = computed(() => subMeshControls.value.length > 0)
const hasMeshRegionControls = computed(() => hasSubMeshControls.value || partControls.value.length > 0)

defineExpose({
  subMeshControls,
  partControls,
  hasSubMeshControls,
  hasMeshRegionControls,
  selectSingleSubMesh,
  setSubMeshVisible,
  selectSinglePart,
  togglePartVisible,
  selectTextureCandidate,
  resetCameraPosition,
  playAnimation,
  pauseAnimation,
  resumeAnimation,
  toggleAnimationPlayback,
  isAnimationPlaying,
  selectedAnimationIndex,
  animationClips,
})

watch([partControls, subMeshControls], () => {
  emit('mesh-regions-change', {
    parts: partControls.value.map(item => ({ ...item })),
    subMeshes: subMeshControls.value.map(item => ({ ...item })),
  })
}, { deep: true })

// ============================================================
// Three.js instance (initialized on mount)
// ============================================================

let renderer: THREE.WebGLRenderer | null = null
let scene: THREE.Scene | null = null
let camera: THREE.PerspectiveCamera | null = null
let composer: EffectComposer | null = null
let outlinePass: OutlinePass | null = null
let controls: OrbitControls | null = null
let animationId: number | null = null
let continuousRenderId: number | null = null
let textureLoader: THREE.TextureLoader | null = null
let gltfLoader: GLTFLoader | null = null
let gltfLoadingManager: THREE.LoadingManager | null = null
let gltfRoot: THREE.Object3D | null = null
let animationMixer: THREE.AnimationMixer | null = null
let activeAnimationAction: THREE.AnimationAction | null = null
let activeGltf: GLTF | null = null
let lastAnimationTime = 0
let cloudTexture: THREE.CanvasTexture | null = null
let gridHelper: THREE.GridHelper | null = null
let raycaster: THREE.Raycaster | null = null
let pointerNdc: THREE.Vector2 | null = null
let highlightedSelectionOverlays: THREE.Object3D[] = []
let activeSubMeshes: Array<{ index_start: number; index_count: number; topology?: number }> = []
let keyboardMoveRenderId: number | null = null
let lastKeyboardMoveTime = 0
const activeMovementKeys = new Set<string>()
const PREVIEW_MESH_FLAG = 'assetFinderPreviewMesh'
const PREVIEW_SUBMESH_OVERLAY_FLAG = 'assetFinderPreviewSubMeshOverlay'
const PART_COLORS = [
  0x018b8d,
  0xd34947,
  0x6ecc54,
  0xf9d46c,
  0x002fa7,
  0x71e2d1,
  0xeb5c20,
  0xc8161d,
  0x470125,
  0x0d3a69,
  0x492d22,
]

/** Resize observer for container, auto-updates Three.js render size and camera aspect on window resize */
let resizeObserver: ResizeObserver | null = null

/**
 * When container size changes, synchronously update Three.js renderer and camera dimensions/aspect ratio.
 *
 * Without this, after window resize the Three.js canvas keeps its initial size,
 * causing the preview to display incorrectly (canvas smaller than container) or stretch distortion (aspect ratio mismatch).
 */
function handleContainerResize(container: HTMLElement) {
  // Skip if renderer or camera not initialized
  if (!renderer || !camera) return

  // Get container current actual pixel dimensions (compared to getBoundingClientRect rounding,
  // devicePixelContentBox is more precise, but getBoundingClientRect is sufficient for Tauri preview needs)
  const rect = container.getBoundingClientRect()
  const width = rect.width || 320
  const height = rect.height || 240

  // Update renderer output size to fill container
  renderer.setSize(width, height)
  if (composer) composer.setSize(width, height)
  if (outlinePass) outlinePass.resolution.set(width, height)

  // Update perspective camera aspect ratio to prevent 3D content from being stretched or compressed
  camera.aspect = width / height
  camera.updateProjectionMatrix()
  requestRender()
}

/** Initialize Three.js scene */
function initScene(container: HTMLElement) {
  const rect = container.getBoundingClientRect()
  const width = rect.width || 320
  const height = rect.height || 240

  // Renderer
  renderer = new THREE.WebGLRenderer({
    antialias: true,
    alpha: true,
  })
  renderer.setSize(width, height)
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2))
  renderer.toneMapping = THREE.ACESFilmicToneMapping
  renderer.toneMappingExposure = 1.0
  renderer.domElement.addEventListener('pointerdown', suppressCanvasMiddleButton, { capture: true })
  renderer.domElement.addEventListener('mousedown', suppressCanvasMiddleButton, { capture: true })
  renderer.domElement.addEventListener('auxclick', suppressCanvasMiddleButton, { capture: true })
  container.appendChild(renderer.domElement)

  // Scene
  scene = new THREE.Scene()
  const previewEnvironment = buildCloudBackground()
  scene.background = previewEnvironment
  if (previewEnvironment instanceof THREE.Texture) {
    scene.environment = previewEnvironment
  }

  // Camera
  camera = new THREE.PerspectiveCamera(45, width / height, 0.1, 1000)
  camera.position.set(2, 1.5, 2)
  camera.lookAt(0, 0, 0)

  // Orbit controls (no auto-rotate)
  controls = new OrbitControls(camera, renderer.domElement)
  controls.enableDamping = true
  controls.dampingFactor = 0.1
  controls.autoRotate = false
  controls.autoRotateSpeed = 0
  controls.target.set(0, 0, 0)
  controls.update()
  controls.addEventListener('change', requestRender)
  controls.addEventListener('start', startContinuousRender)
  controls.addEventListener('end', stopContinuousRender)
  raycaster = new THREE.Raycaster()
  pointerNdc = new THREE.Vector2()

  addCinematicLighting(scene)

  composer = new EffectComposer(renderer)
  composer.setSize(width, height)
  composer.addPass(new RenderPass(scene, camera))
  outlinePass = new OutlinePass(new THREE.Vector2(width, height), scene, camera)
  outlinePass.visibleEdgeColor.set(SUBMESH_OUTLINE_COLOR)
  outlinePass.hiddenEdgeColor.set(SUBMESH_OUTLINE_COLOR)
  outlinePass.edgeStrength = 7
  outlinePass.edgeGlow = 0
  outlinePass.edgeThickness = 1.6
  outlinePass.pulsePeriod = 0
  composer.addPass(outlinePass)
  composer.addPass(new OutputPass())

  // Grid helper (visual aid)
  gridHelper = new THREE.GridHelper(4, 20, PREVIEW_DEFAULT_COLORS.gridMajor, PREVIEW_DEFAULT_COLORS.gridMinor)
  gridHelper.visible = false
  scene.add(gridHelper)

  requestRender()
}

function addCinematicLighting(targetScene: THREE.Scene) {
  const ambientLight = new THREE.AmbientLight(0xffffff, 0.22)
  targetScene.add(ambientLight)

  const hemisphereLight = new THREE.HemisphereLight(PREVIEW_DEFAULT_COLORS.hemisphereSky, PREVIEW_DEFAULT_COLORS.hemisphereGround, 0.7)
  targetScene.add(hemisphereLight)

  const keyLight = new THREE.DirectionalLight(PREVIEW_DEFAULT_COLORS.keyLight, 2.0)
  keyLight.position.set(4.5, 6.5, 4)
  targetScene.add(keyLight)

  const fillLight = new THREE.DirectionalLight(PREVIEW_DEFAULT_COLORS.fillLight, 0.72)
  fillLight.position.set(-5, 3, 2.5)
  targetScene.add(fillLight)

  const rimLight = new THREE.DirectionalLight(PREVIEW_DEFAULT_COLORS.rimLight, 1.35)
  rimLight.position.set(-3.5, 4.2, -5)
  targetScene.add(rimLight)
}

/** Render one frame on demand. Static previews should not keep the GPU busy. */
function requestRender() {
  if (continuousRenderId !== null) return
  if (animationId !== null) return
  animationId = requestAnimationFrame(() => {
    animationId = null
    if (controls) controls.update()
    renderFrame()
  })
}

function renderFrame() {
  if (renderer && scene && camera) {
    if (composer) {
      composer.render()
    } else {
      renderer.render(scene, camera)
    }
  }
}

function resetCameraPosition() {
  if (!camera || !controls || !scene) return

  const { box, hasBounds } = computeRenderableMeshBounds(false)
  const fallback = hasBounds ? { box, hasBounds } : computeRenderableMeshBounds(true)

  const center = fallback.hasBounds ? fallback.box.getCenter(new THREE.Vector3()) : new THREE.Vector3(0, 0, 0)
  const size = fallback.hasBounds ? fallback.box.getSize(new THREE.Vector3()) : new THREE.Vector3(3, 3, 3)
  const maxDim = Math.max(size.x, size.y, size.z, 1)
  const fov = THREE.MathUtils.degToRad(camera.fov)
  const distance = Math.max(2.8, (maxDim / (2 * Math.tan(fov / 2))) * 1.75)
  const direction = new THREE.Vector3(1, 0.72, 1).normalize()

  camera.near = Math.max(0.01, distance / 100)
  camera.far = Math.max(1000, distance * 100)
  camera.position.copy(center).addScaledVector(direction, distance)
  camera.lookAt(center)
  camera.updateProjectionMatrix()
  controls.target.copy(center)
  controls.update()
  requestRender()
}

function computeRenderableMeshBounds(includeHidden: boolean) {
  const box = new THREE.Box3()
  let hasBounds = false
  if (!scene) return { box, hasBounds }

  scene.updateMatrixWorld(true)
  scene.traverse((object) => {
    if (!object.userData[PREVIEW_MESH_FLAG] || object.userData[PREVIEW_SUBMESH_OVERLAY_FLAG]) return
    if (!(object instanceof THREE.Mesh)) return
    if (!includeHidden && !object.visible) return
    const geometry = object.geometry
    if (!geometry) return
    if (!geometry.boundingBox) geometry.computeBoundingBox()
    if (!geometry.boundingBox || geometry.boundingBox.isEmpty()) return
    const objectBox = geometry.boundingBox.clone().applyMatrix4(object.matrixWorld)
    if (objectBox.isEmpty()) return
    box.union(objectBox)
    hasBounds = true
  })

  return { box, hasBounds }
}

function startContinuousRender() {
  if (continuousRenderId !== null) return
  lastAnimationTime = performance.now()
  const tick = () => {
    const now = performance.now()
    const delta = Math.min(0.05, (now - lastAnimationTime) / 1000)
    lastAnimationTime = now
    if (animationMixer && isAnimationPlaying.value) {
      animationMixer.update(delta)
    }
    if (controls) controls.update()
    renderFrame()
    continuousRenderId = requestAnimationFrame(tick)
  }
  continuousRenderId = requestAnimationFrame(tick)
}

function stopContinuousRender() {
  if (isAnimationPlaying.value) return
  if (continuousRenderId !== null) {
    cancelAnimationFrame(continuousRenderId)
    continuousRenderId = null
  }
  requestRender()
}

function stopViewerInteractionRender() {
  if (!isAnimationPlaying.value) {
    stopContinuousRender()
  }
}

function handleGlobalPointerEnd(event: PointerEvent | MouseEvent) {
  if (event.button === 1 || continuousRenderId !== null) {
    stopViewerInteractionRender()
  }
}

function handleGlobalInteractionCancel() {
  activeMovementKeys.clear()
  stopKeyboardCameraMove()
  stopViewerInteractionRender()
}

function suppressCanvasMiddleButton(event: MouseEvent | PointerEvent) {
  if (event.button !== 1) return
  event.preventDefault()
  event.stopPropagation()
  event.stopImmediatePropagation()
  stopViewerInteractionRender()
}

/** Dispose Three.js resources */
function disposeScene() {
  if (animationId !== null) {
    cancelAnimationFrame(animationId)
    animationId = null
  }
  if (continuousRenderId !== null) {
    cancelAnimationFrame(continuousRenderId)
    continuousRenderId = null
  }
  if (keyboardMoveRenderId !== null) {
    cancelAnimationFrame(keyboardMoveRenderId)
    keyboardMoveRenderId = null
  }
  activeMovementKeys.clear()
  if (controls) {
    controls.removeEventListener('change', requestRender)
    controls.removeEventListener('start', startContinuousRender)
    controls.removeEventListener('end', stopContinuousRender)
    controls.dispose()
    controls = null
  }
  if (composer) {
    composer.dispose()
    composer = null
  }
  outlinePass = null
  if (renderer) {
    renderer.dispose()
    if (renderer.domElement.parentElement) {
      renderer.domElement.parentElement.removeChild(renderer.domElement)
    }
    renderer.domElement.removeEventListener('pointerdown', suppressCanvasMiddleButton, { capture: true })
    renderer.domElement.removeEventListener('mousedown', suppressCanvasMiddleButton, { capture: true })
    renderer.domElement.removeEventListener('auxclick', suppressCanvasMiddleButton, { capture: true })
    renderer = null
  }
  stopAnimationPlayback()
  disposeGltfRoot()
  if (cloudTexture) {
    cloudTexture.dispose()
    cloudTexture = null
  }
  highlightedSelectionOverlays = []
  raycaster = null
  pointerNdc = null
  gridHelper = null
  scene = null
  camera = null
}

/** Build Three.js mesh from vertex/index data */
function buildMeshFromData(data: {
  vertices: number[]
  indices: number[]
  normals?: number[]
  uvs?: number[]
  sub_meshes?: Array<{ index_start: number; index_count: number; topology?: number }>
  selected_submesh_indices?: number[]
  parts?: Array<{
    name?: string
    asset_name?: string
    bundle_path: string
    path_id: string
    vertices: number[]
    indices: number[]
    normals?: number[]
    uvs?: number[]
    sub_meshes?: Array<{ index_start: number; index_count: number; topology?: number }>
  }>
  diffuse_texture_path?: string
  glb_url?: string
  glb_path?: string
  mesh_part_names?: string[]
}) {
  if (!scene) return

  loadState.value = 'loading'

  try {
    if (data.glb_url || data.glb_path) {
      void buildGltfPreview(data.glb_url || data.glb_path || '')
      return
    }
    const parts = data.parts
    if (parts?.length) {
      buildMeshPartsFromData({ parts })
      return
    }
    activeSubMeshes = data.sub_meshes || []
    partControls.value = []
    syncSubMeshControls(data)

    const { vertices, indices } = data

    if (!vertices || vertices.length < 3) {
      throw new Error(`Insufficient vertex data: ${vertices?.length || 0}`)
    }

    const vertexCount = vertices.length / 3
    if (vertexCount > MAX_PREVIEW_VERTICES || (indices?.length || 0) > MAX_PREVIEW_INDICES) {
      throw new Error(`Mesh too large for interactive preview: ${Math.floor(vertexCount)} vertices, ${indices?.length || 0} indices`)
    }

    // Create BufferGeometry
    const geometry = new THREE.BufferGeometry()
    const positions = new Float32Array(vertices)
    geometry.setAttribute('position', new THREE.BufferAttribute(positions, 3))

    if (data.normals && data.normals.length >= vertexCount * 3) {
      geometry.setAttribute('normal', new THREE.BufferAttribute(new Float32Array(data.normals.slice(0, vertexCount * 3)), 3))
    }

    if (data.uvs && data.uvs.length >= vertexCount * 2) {
      geometry.setAttribute('uv', new THREE.BufferAttribute(buildUvBuffer(data.uvs, vertexCount), 2))
    }

    // Validate index legality: index values must not exceed vertex count.
    // If maxIdx >= vertexCount, iterating all indices would access out of bounds on the vertex array.
    // Common cause: indices come from m_IndexBuffer (global scope) but vertices come from local parsing (sub-range).
    // In this case, degrade to non-indexed rendering: every 3 consecutive vertices = 1 triangle.
    if (indices && indices.length >= 3) {
      const maxIdx = getMaxIndex(indices)
      if (maxIdx < vertexCount) {
        geometry.setIndex(new THREE.BufferAttribute(new Uint32Array(indices), 1))
      } else {
        emit('geometry-error', { maxIdx, vertexCount, indexCount: indices.length })
      }
    }

    if (!geometry.getAttribute('normal') && shouldComputeNormals(vertexCount, indices?.length || 0)) {
      geometry.computeVertexNormals()
    }

    const material = buildPreviewMaterials(data)
    applySubMeshGroups(geometry, data, Array.isArray(material) ? material.length : 1, visibleSubMeshIndices(data))

    // Remove old model
    clearModel()

    // Add main mesh. Selection outlines are handled separately so the model itself stays clean.
    const mesh = new THREE.Mesh(geometry, material)
    mesh.userData[PREVIEW_MESH_FLAG] = true
    mesh.userData.subMeshCount = data.sub_meshes?.length || 0
    scene!.add(mesh)

    // Center and scale
    const box = new THREE.Box3().setFromObject(mesh)
    const center = box.getCenter(new THREE.Vector3())
    const size = box.getSize(new THREE.Vector3())
    const maxDim = Math.max(size.x, size.y, size.z)

    if (maxDim > 0 && isFinite(maxDim)) {
      const s = 3.0 / maxDim
      mesh.scale.set(s, s, s)
      mesh.position.set(-center.x * s, -center.y * s, -center.z * s)
    }

    loadState.value = 'ready'

    resetCameraPosition()
    updateSubMeshOverlay(data)
    requestRender()
  } catch (e) {
    console.error('Mesh build failed:', e)
    loadState.value = 'error'
    loadError.value = String(e)
  }
}

function buildMeshPartsFromData(data: {
  parts: Array<{
    name?: string
    asset_name?: string
    bundle_path: string
    path_id: string
    vertices: number[]
    indices: number[]
    normals?: number[]
    uvs?: number[]
    sub_meshes?: Array<{ index_start: number; index_count: number }>
  }>
}) {
  if (!scene) return
  activeSubMeshes = []
  subMeshControls.value = []
  const previousVisibility = new Map(partControls.value.map((part) => [part.key, part.visible]))
  const previousSelected = new Set(partControls.value.filter((part) => part.selected).map((part) => part.key))
  partControls.value = data.parts.map((part, index) => {
    const key = meshPartKey(part, index)
    const name = meshDisplayName(part, index)
    const defaultVisible = isDefaultMeshRegionVisible(name)
    return {
      key,
      index,
      name,
      displayName: meshRegionDisplayName(name, index, 'Mesh Part'),
      color: colorCss(PART_COLORS[index % PART_COLORS.length]),
      visible: previousVisibility.get(key) ?? defaultVisible,
      selected: previousSelected.size > 0 ? previousSelected.has(key) : false,
      bundlePath: part.bundle_path,
      pathId: part.path_id,
      className: 'Mesh',
    }
  })
  ensureSelectedVisiblePart()
  clearModel()

  const group = new THREE.Group()
  group.userData[PREVIEW_MESH_FLAG] = true
  scene.add(group)

  let added = 0
  for (let index = 0; index < data.parts.length; index++) {
    const part = data.parts[index]
    const mesh = createPartMesh(part, index)
    if (!mesh) continue
    group.add(mesh)
    added += 1
  }

  if (added === 0) {
    throw new Error('No valid Animator mesh parts to preview')
  }

  centerAndScaleObject(group)
  loadState.value = 'ready'

  resetCameraPosition()
  updatePartSelectionOverlay()
  requestRender()
}

async function buildGltfPreview(glbUrl: string) {
  if (!scene) return
  if (!glbUrl) {
    loadState.value = 'error'
    loadError.value = 'GLB path is empty'
    return
  }

  try {
    clearModel()
    disposeGltfRoot()
    activeSubMeshes = []
    partControls.value = []
    animationClips.value = []
    selectedAnimationIndex.value = 0
    isAnimationPlaying.value = false

    if (!gltfLoadingManager) {
      gltfLoadingManager = new THREE.LoadingManager()
      gltfLoadingManager.setURLModifier(resolveGltfResourceUrl)
    }
    if (!gltfLoader) gltfLoader = new GLTFLoader(gltfLoadingManager)
    const resourcePath = glbUrl.slice(0, Math.max(0, glbUrl.lastIndexOf('/') + 1))
    gltfLoader.setResourcePath(resourcePath)
    const gltf = await gltfLoader.loadAsync(glbUrl)
    activeGltf = gltf
    gltfRoot = gltf.scene
    gltfRoot.userData[PREVIEW_MESH_FLAG] = true

    normalizeGltfMaterials(gltfRoot)
    scene.add(gltfRoot)
    centerAndScaleObject(gltfRoot)
    buildGltfPartControls(gltfRoot)
    setupGltfAnimations(gltf)

    loadState.value = 'ready'
    resetCameraPosition()
    requestRender()
  } catch (e) {
    console.error('GLB load failed:', e)
    loadState.value = 'error'
    loadError.value = String(e)
    stopAnimationPlayback()
  }
}

function resolveGltfResourceUrl(url: string) {
  if (!url) return url
  const requestedFileName = resourceFileName(url)
  const match = props.geometryData?.texture_candidates?.find((candidate) => {
    return resourceFileName(candidate.png_path || '') === requestedFileName
  })
  return match?.png_path || url
}

function resourceFileName(url: string) {
  const withoutQuery = url.split(/[?#]/, 1)[0]
  const rawName = withoutQuery.replace(/\\/g, '/').split('/').pop() || withoutQuery
  try {
    return decodeURIComponent(rawName)
  } catch {
    return rawName
  }
}

function normalizeGltfMaterials(root: THREE.Object3D) {
  root.traverse((child) => {
    const mesh = child as THREE.Mesh
    if (!(mesh instanceof THREE.Mesh)) return
    mesh.frustumCulled = false
    const materials = Array.isArray(mesh.material) ? mesh.material : [mesh.material]
    for (const material of materials) {
      const maybeStandard = material as THREE.MeshStandardMaterial
      if (maybeStandard.map) {
        maybeStandard.map.colorSpace = THREE.SRGBColorSpace
        maybeStandard.map.flipY = false
      }
      maybeStandard.side = THREE.DoubleSide
      maybeStandard.needsUpdate = true
    }
  })
}

function buildGltfPartControls(root: THREE.Object3D) {
  const previousVisibility = new Map(partControls.value.map((part) => [part.key, part.visible]))
  const previousSelected = new Set(partControls.value.filter((part) => part.selected).map((part) => part.key))
  const meshPartNames = props.geometryData?.mesh_part_names || []
  const meshParts = props.geometryData?.mesh_parts || props.geometryData?.parts || []
  const meshNodes: THREE.Object3D[] = []
  root.traverse((child) => {
    if (child instanceof THREE.Mesh || child instanceof THREE.SkinnedMesh) {
      meshNodes.push(child)
    }
  })
  partControls.value = meshNodes.map((mesh, index) => {
    const key = mesh.uuid
    mesh.userData[PREVIEW_MESH_FLAG] = true
    mesh.userData.partKey = key
    mesh.userData.partIndex = index
    const sourceName = meshPartNames[index]?.trim() || mesh.name?.trim() || `mesh_${index + 1}`
    const sourcePart = meshParts[index]
    const sourcePartClassName = sourcePart && 'class_name' in sourcePart ? sourcePart.class_name : undefined
    const visible = previousVisibility.get(key) ?? isDefaultMeshRegionVisible(sourceName)
    mesh.visible = visible
    mesh.name = sourceName
    return {
      key,
      index,
      name: sourceName,
      displayName: sourceName,
      color: colorCss(PART_COLORS[index % PART_COLORS.length]),
      visible,
      selected: previousSelected.size > 0 ? previousSelected.has(key) : false,
      bundlePath: sourcePart?.bundle_path,
      pathId: sourcePart?.path_id,
      className: sourcePartClassName || (sourcePart ? 'Mesh' : undefined),
    }
  })
  ensureSelectedVisiblePart()
  subMeshControls.value = []
  updatePartSelectionOverlay()
}

function setupGltfAnimations(gltf: GLTF) {
  if (!gltf.animations.length || !gltfRoot) {
    animationMixer = null
    activeAnimationAction = null
    return
  }
  animationMixer = new THREE.AnimationMixer(gltfRoot)
  animationClips.value = gltf.animations.map((clip, index) => ({
    index,
    name: clip.name || `Animation ${index + 1}`,
    duration: clip.duration,
  }))
  selectedAnimationIndex.value = preferredAnimationClipIndex(
    gltf.animations,
    gltfRoot,
    props.preferredAnimationName,
  )
  playAnimation(selectedAnimationIndex.value)
}

function preferredAnimationClipIndex(
  clips: THREE.AnimationClip[],
  root: THREE.Object3D,
  preferredAnimationName?: string,
) {
  const movingClipIndices = clips
    .map((clip, index) => ({ clip, index }))
    .filter(({ clip }) => animationClipHasMotion(clip))
  if (!movingClipIndices.length) return 0

  const preferredKey = normalizeAnimationName(preferredAnimationName || '')
  if (preferredKey) {
    const preferredMatch = movingClipIndices.find(({ clip }) => {
      const clipKey = normalizeAnimationName(clip.name)
      return clipKey === preferredKey || clipKey.includes(preferredKey) || preferredKey.includes(clipKey)
    })
    if (preferredMatch) return preferredMatch.index
  }

  const skinnedBoneNames = collectSkinnedBoneNames(root)
  const skinMotionMatch = movingClipIndices.find(({ clip }) => animationClipTargetsSkinnedBones(clip, skinnedBoneNames))
  if (skinMotionMatch) return skinMotionMatch.index

  const assetKey = normalizeAnimationName(props.assetName)
  const assetMatch = movingClipIndices.find(({ clip }) => {
    const clipKey = normalizeAnimationName(clip.name)
    return Boolean(assetKey && (clipKey.includes(assetKey) || assetKey.includes(clipKey)))
  })
  return assetMatch?.index ?? movingClipIndices[0].index
}

function collectSkinnedBoneNames(root: THREE.Object3D) {
  const names = new Set<string>()
  root.traverse((child) => {
    if (!(child instanceof THREE.SkinnedMesh)) return
    for (const bone of child.skeleton.bones) {
      if (bone.name) names.add(bone.name)
    }
  })
  return names
}

function animationClipTargetsSkinnedBones(clip: THREE.AnimationClip, skinnedBoneNames: Set<string>) {
  if (!skinnedBoneNames.size) return false
  return clip.tracks.some((track) => {
    const targetName = THREE.PropertyBinding.parseTrackName(track.name).nodeName
    return Boolean(targetName && skinnedBoneNames.has(targetName) && trackHasMotion(track))
  })
}

function normalizeAnimationName(name: string) {
  return name.toLowerCase().replace(/[^a-z0-9]+/g, '')
}

function animationClipHasMotion(clip: THREE.AnimationClip) {
  return clip.tracks.some((track) => trackHasMotion(track))
}

function trackHasMotion(track: THREE.KeyframeTrack) {
  const itemSize = track.getValueSize()
  const values = track.values
  if (itemSize <= 0 || values.length <= itemSize) return false

  for (let offset = itemSize; offset + itemSize <= values.length; offset += itemSize) {
    for (let component = 0; component < itemSize; component++) {
      if (Math.abs(values[offset + component] - values[component]) > 1e-4) {
        return true
      }
    }
  }
  return false
}

function playAnimation(index = selectedAnimationIndex.value) {
  if (!animationMixer || !activeGltf?.animations[index]) return
  if (activeAnimationAction) {
    activeAnimationAction.stop()
    activeAnimationAction = null
  }
  selectedAnimationIndex.value = index
  activeAnimationAction = animationMixer.clipAction(activeGltf.animations[index])
  activeAnimationAction.reset()
  activeAnimationAction.play()
  isAnimationPlaying.value = true
  emit('animation-playback-change', {
    playing: true,
    clipIndex: selectedAnimationIndex.value,
    clipName: activeGltf.animations[index]?.name || null,
    targetedBoneNames: animationClipTargetNames(activeGltf.animations[index]),
  })
  startContinuousRender()
}

function pauseAnimation() {
  if (!activeAnimationAction) return
  activeAnimationAction.paused = true
  isAnimationPlaying.value = false
  emit('animation-playback-change', {
    playing: false,
    clipIndex: selectedAnimationIndex.value,
    clipName: activeGltf?.animations[selectedAnimationIndex.value]?.name || null,
    targetedBoneNames: activeGltf?.animations[selectedAnimationIndex.value]
      ? animationClipTargetNames(activeGltf.animations[selectedAnimationIndex.value])
      : [],
  })
  stopContinuousRender()
}

function resumeAnimation() {
  if (!activeAnimationAction) {
    playAnimation(selectedAnimationIndex.value)
    return
  }
  activeAnimationAction.paused = false
  isAnimationPlaying.value = true
  emit('animation-playback-change', {
    playing: true,
    clipIndex: selectedAnimationIndex.value,
    clipName: activeGltf?.animations[selectedAnimationIndex.value]?.name || null,
    targetedBoneNames: activeGltf?.animations[selectedAnimationIndex.value]
      ? animationClipTargetNames(activeGltf.animations[selectedAnimationIndex.value])
      : [],
  })
  startContinuousRender()
}

function toggleAnimationPlayback() {
  if (!animationClips.value.length) return
  if (isAnimationPlaying.value) {
    pauseAnimation()
  } else {
    resumeAnimation()
  }
}

function stopAnimationPlayback() {
  if (activeAnimationAction) {
    activeAnimationAction.stop()
    activeAnimationAction = null
  }
  if (animationMixer) {
    animationMixer.stopAllAction()
    animationMixer = null
  }
  isAnimationPlaying.value = false
  animationClips.value = []
  emit('animation-playback-change', { playing: false, clipIndex: null, clipName: null, targetedBoneNames: [] })
}

function animationClipTargetNames(clip: THREE.AnimationClip) {
  const names = new Set<string>()
  for (const track of clip.tracks) {
    const targetName = THREE.PropertyBinding.parseTrackName(track.name).nodeName
    if (targetName) names.add(targetName)
  }
  return [...names]
}

function disposeGltfRoot() {
  activeGltf = null
  gltfRoot = null
}

function meshPartKey(
  part: MeshPreviewPart,
  index: number,
) {
  return `${index}:${part.bundle_path}:${part.path_id}`
}

function meshDisplayName(part: MeshPreviewPart, index: number) {
  return part.asset_name?.trim() || part.name?.trim() || `mesh_${part.path_id || index + 1}`
}

function meshRegionDisplayName(name: string, index: number, fallbackPrefix: 'Mesh Part' | 'Submesh') {
  const trimmed = name.trim()
  if (!trimmed) return `${fallbackPrefix} ${index + 1}`
  return trimmed
}

function isLevelSuffixName(name: string) {
  return /(?:^|[_\-. ])L\d+(?:\s*\/\s*Submesh\s+\d+)?$/i.test(name.trim())
}

function isDefaultMeshRegionVisible(name: string) {
  return !isLevelSuffixName(name)
}

function ensureSelectedVisiblePart() {
  if (!partControls.value.length) return
  if (partControls.value.some(part => part.selected && part.visible)) return
  const fallback = partControls.value.find(part => part.visible) || partControls.value[0]
  partControls.value = partControls.value.map(part => ({
    ...part,
    selected: part.key === fallback.key,
  }))
}

function ensureSelectedVisibleSubMesh() {
  if (!subMeshControls.value.length) return
  if (subMeshControls.value.some(subMesh => subMesh.selected && subMesh.visible)) return
  const fallback = subMeshControls.value.find(subMesh => subMesh.visible) || subMeshControls.value[0]
  subMeshControls.value = subMeshControls.value.map(subMesh => ({
    ...subMesh,
    selected: subMesh.index === fallback.index,
  }))
}

function createPartMesh(
  part: {
    name?: string
    asset_name?: string
    bundle_path: string
    path_id: string
    vertices: number[]
    indices: number[]
    normals?: number[]
    uvs?: number[]
  },
  partIndex: number,
) {
  const vertices = part.vertices || []
  const indices = part.indices || []
  if (vertices.length < 3) return null

  const vertexCount = vertices.length / 3
  if (vertexCount > MAX_PREVIEW_VERTICES || indices.length > MAX_PREVIEW_INDICES) {
    throw new Error(`Mesh part too large for interactive preview: ${part.name}`)
  }

  const geometry = new THREE.BufferGeometry()
  geometry.setAttribute('position', new THREE.BufferAttribute(new Float32Array(vertices), 3))

  if (part.normals && part.normals.length >= vertexCount * 3) {
    geometry.setAttribute('normal', new THREE.BufferAttribute(new Float32Array(part.normals.slice(0, vertexCount * 3)), 3))
  }

  if (part.uvs && part.uvs.length >= vertexCount * 2) {
    geometry.setAttribute('uv', new THREE.BufferAttribute(buildUvBuffer(part.uvs, vertexCount), 2))
  }

  if (indices.length >= 3) {
    const maxIdx = getMaxIndex(indices)
    if (maxIdx < vertexCount) {
      geometry.setIndex(new THREE.BufferAttribute(new Uint32Array(indices), 1))
    } else {
      emit('geometry-error', { maxIdx, vertexCount, indexCount: indices.length })
    }
  }

  if (!geometry.getAttribute('normal') && shouldComputeNormals(vertexCount, indices.length)) {
    geometry.computeVertexNormals()
  }

  const shouldUseSelectedTexture = props.geometryData?.diffuse_texture_user_selected &&
    partControls.value[partIndex]?.selected
  const material = createPartMaterial(
    partIndex,
    shouldUseSelectedTexture ? props.geometryData?.diffuse_texture_path : undefined,
    shouldUseSelectedTexture,
  )
  const mesh = new THREE.Mesh(geometry, material)
  mesh.name = meshDisplayName(part, partIndex)
  mesh.userData[PREVIEW_MESH_FLAG] = true
  mesh.userData.partIndex = partIndex
  mesh.userData.partKey = meshPartKey(part, partIndex)
  mesh.userData.pathId = part.path_id
  mesh.visible = partControls.value[partIndex]?.visible ?? true
  return mesh
}

function createPartMaterial(partIndex: number, texturePath?: string, flipY = false) {
  const diffuseTexture = createDiffuseTexture(texturePath, flipY)
  return new THREE.MeshStandardMaterial({
    color: diffuseTexture ? 0xffffff : PART_COLORS[partIndex % PART_COLORS.length],
    map: diffuseTexture || null,
    metalness: 0.05,
    roughness: 0.38,
    envMapIntensity: 0.75,
    side: THREE.DoubleSide,
  })
}

function colorCss(color: number) {
  return `#${color.toString(16).padStart(6, '0')}`
}

function toggleMeshPart(partKey: string) {
  const nextControls = partControls.value.map((part) => (
    part.key === partKey ? { ...part, visible: !part.visible } : part
  ))
  partControls.value = nextControls
  const next = nextControls.find((part) => part.key === partKey)
  if (!scene || !next) return
  scene.traverse((child) => {
    if (child.userData?.partKey === partKey) {
      child.visible = next.visible
    }
  })
  requestRender()
}

function selectedPartKeys() {
  const selected = partControls.value.filter(item => item.selected).map(item => item.key)
  if (selected.length > 0) return selected
  return partControls.value.length > 0 ? [partControls.value[0].key] : []
}

function selectSinglePart(partKey: string) {
  partControls.value = partControls.value.map(item => ({
    ...item,
    selected: item.key === partKey,
  }))
  updatePartSelectionOverlay()
  requestRender()
}

function togglePartVisible(partKey: string) {
  toggleMeshPart(partKey)
  updatePartSelectionOverlay()
}

function syncSubMeshControls(data: {
  sub_meshes?: Array<{ index_start: number; index_count: number; topology?: number }>
  selected_submesh_indices?: number[]
  mesh_part_names?: string[]
}) {
  const subMeshes = data.sub_meshes || []
  if (subMeshes.length <= 1) {
    subMeshControls.value = []
    return
  }
  const previousByIndex = new Map(subMeshControls.value.map((item) => [item.index, item]))
  const previousSelected = subMeshControls.value
    .filter((item) => item.selected)
    .map((item) => item.index)
  const selectedSet = new Set(
    data.selected_submesh_indices?.length
      ? data.selected_submesh_indices
      : previousSelected.length
        ? previousSelected
      : [0],
  )
  subMeshControls.value = subMeshes.map((subMesh, index) => {
    const name = data.mesh_part_names?.[index]?.trim() || `Submesh ${index + 1}`
    const defaultVisible = subMesh.index_count > 0 && isDefaultMeshRegionVisible(name)
    return {
      key: `submesh:${index}`,
      index,
      name,
      displayName: meshRegionDisplayName(name, index, 'Submesh'),
      visible: previousByIndex.get(index)?.visible ?? defaultVisible,
      selected: selectedSet.has(index),
      color: colorCss(PART_COLORS[index % PART_COLORS.length]),
    }
  })
  ensureSelectedVisibleSubMesh()
}

function selectedSubMeshIndices() {
  const selected = subMeshControls.value.filter(item => item.selected).map(item => item.index)
  if (selected.length > 0) return selected
  return subMeshControls.value.length > 0 ? [subMeshControls.value[0].index] : []
}

function updateGeometrySelectedSubMeshes() {
  if (!props.geometryData) return
  props.geometryData.selected_submesh_indices = selectedSubMeshIndices()
}

function selectSingleSubMesh(index: number) {
  subMeshControls.value = subMeshControls.value.map(item => ({
    ...item,
    selected: item.index === index,
  }))
  updateGeometrySelectedSubMeshes()
  updateSubMeshOverlay(props.geometryData || null)
  requestRender()
}

function refreshSubMeshPresentation() {
  if (props.geometryData && scene && loadState.value === 'ready') {
    updatePreviewMaterials(props.geometryData)
    updateSubMeshOverlay(props.geometryData)
  }
  requestRender()
}

function setSubMeshVisible(index: number, visible: boolean) {
  subMeshControls.value = subMeshControls.value.map(item => {
    if (item.index !== index) return item
    return {
      ...item,
      visible,
    }
  })
  refreshSubMeshPresentation()
}

function toggleSelectedSubMeshVisibility() {
  const selectedIndices = selectedSubMeshIndices()
  if (!selectedIndices.length) return
  const selectedSet = new Set(selectedIndices)
  const selectedControls = subMeshControls.value.filter(item => selectedSet.has(item.index))
  if (!selectedControls.length) return
  const nextVisible = !selectedControls.every(item => item.visible)
  subMeshControls.value = subMeshControls.value.map(item => (
    selectedSet.has(item.index) ? { ...item, visible: nextVisible } : item
  ))
  refreshSubMeshPresentation()
}

function toggleSelectedMeshRegionVisibility() {
  if (hasSubMeshControls.value) {
    toggleSelectedSubMeshVisibility()
    return
  }
  const partKeys = selectedPartKeys()
  if (!partKeys.length) return
  const selectedSet = new Set(partKeys)
  const selectedControls = partControls.value.filter(item => selectedSet.has(item.key))
  if (!selectedControls.length) return
  const nextVisible = !selectedControls.every(item => item.visible)
  partControls.value = partControls.value.map(item => (
    selectedSet.has(item.key) ? { ...item, visible: nextVisible } : item
  ))
  if (scene) {
    scene.traverse((child) => {
      const partKey = child.userData?.partKey
      if (partKey && selectedSet.has(partKey)) {
        child.visible = nextVisible
      }
    })
  }
  updatePartSelectionOverlay()
  requestRender()
}

function handleSubMeshListClick(index: number) {
  selectSingleSubMesh(index)
}

function buildSelectedTextureCandidateKey(candidate: PreviewTextureCandidate) {
  return [
    candidate.candidate_index ?? '',
    candidate.texture_bundle_path || '',
    candidate.texture_path_id || '',
    candidate.material_index,
    candidate.slot_name || '',
  ].join('::')
}

function selectTextureCandidate(candidate: PreviewTextureCandidate) {
  if (!props.geometryData) return null
  lastAppliedTextureCandidateKey.value = buildSelectedTextureCandidateKey(candidate)
  const selected_submesh_indices = selectedSubMeshIndices()
  props.geometryData.diffuse_texture_path = candidate.png_path
  props.geometryData.diffuse_texture_path_id = candidate.texture_path_id
  props.geometryData.diffuse_texture_bundle_path = candidate.texture_bundle_path
  props.geometryData.diffuse_texture_name = candidate.texture_name
  props.geometryData.diffuse_material_name = candidate.material_name
  props.geometryData.diffuse_material_index = candidate.material_index
  props.geometryData.diffuse_slot_name = candidate.slot_name
  props.geometryData.diffuse_texture_user_selected = true
  props.geometryData.selected_submesh_indices = selected_submesh_indices
  updatePreviewMaterials(props.geometryData)
  return { selected_submesh_indices }
}

function centerAndScaleObject(object: THREE.Object3D) {
  const box = new THREE.Box3().setFromObject(object)
  const center = box.getCenter(new THREE.Vector3())
  const size = box.getSize(new THREE.Vector3())
  const maxDim = Math.max(size.x, size.y, size.z)

  if (maxDim > 0 && isFinite(maxDim)) {
    const s = 3.0 / maxDim
    object.scale.set(s, s, s)
    object.position.set(-center.x * s, -center.y * s, -center.z * s)
  }
}

function getMaxIndex(indices: number[]) {
  let maxIdx = 0
  for (let i = 0; i < indices.length; i++) {
    if (indices[i] > maxIdx) maxIdx = indices[i]
  }
  return maxIdx
}

function shouldComputeNormals(vertexCount: number, indexCount: number) {
  return vertexCount <= MAX_PREVIEW_VERTICES && indexCount <= MAX_COMPUTED_NORMAL_INDICES
}

function buildPreviewMaterials(data: PreviewMaterialData): THREE.Material | THREE.Material[] {
  const selectedSubMeshes = new Set(data.selected_submesh_indices?.length ? data.selected_submesh_indices : [0])
  const selectedOverride = findSelectedTextureOverride(data)
  const maxMaterialIndex = Math.max(
    0,
    (data.sub_meshes?.length || 1) - 1,
    ...((data.texture_candidates || []).map(candidate => candidate.material_index)),
  )
  const materials: THREE.Material[] = []
  for (let index = 0; index <= maxMaterialIndex; index++) {
    const replacement = selectedOverride && selectedSubMeshes.has(index)
      ? selectedOverride.png_path
      : undefined
    materials.push(createPreviewMaterial(replacement, Boolean(replacement)))
  }
  return materials.length === 1 ? materials[0] : materials
}

function updatePreviewMaterials(data: PreviewMaterialData & {
  indices: number[]
  sub_meshes?: Array<{ index_start: number; index_count: number; topology?: number }>
  glb_path?: string
  glb_url?: string
}) {
  if (data.glb_path || data.glb_url) {
    updateGltfPreviewMaterials(data)
    return
  }
  if (partControls.value.length > 0) {
    updatePartPreviewMaterials(data)
    return
  }
  const mesh = findPreviewMesh()
  if (!mesh) return

  const nextMaterial = buildPreviewMaterials(data)
  const previousMaterial = mesh.material
  mesh.material = nextMaterial
  applySubMeshGroups(mesh.geometry, data, Array.isArray(nextMaterial) ? nextMaterial.length : 1, visibleSubMeshIndices(data))
  disposeMaterials(previousMaterial)
  requestRender()
}

function updatePartPreviewMaterials(data: PreviewMaterialData) {
  const selectedOverride = findSelectedTextureOverride(data)
  const selectedParts = new Set(selectedPartKeys())
  const disposedMaterials = new Set<THREE.Material>()
  for (const mesh of findPreviewMeshes()) {
    const partKey = mesh.userData.partKey
    const partIndex = typeof mesh.userData.partIndex === 'number' ? mesh.userData.partIndex : 0
    const nextTexture = selectedOverride && partKey && selectedParts.has(partKey)
      ? selectedOverride.png_path
      : undefined
    const previousMaterial = mesh.material
    mesh.material = createPartMaterial(partIndex, nextTexture, Boolean(nextTexture))
    if (Array.isArray(previousMaterial)) {
      previousMaterial.forEach(material => disposeMaterialOnce(material, disposedMaterials))
    } else {
      disposeMaterialOnce(previousMaterial, disposedMaterials)
    }
  }
  updatePartSelectionOverlay()
  requestRender()
}

function updateGltfPreviewMaterials(data: PreviewMaterialData) {
  if (!gltfRoot) return

  const selectedOverride = findSelectedTextureOverride(data)
  const selectedSubMeshes = new Set(data.selected_submesh_indices?.length ? data.selected_submesh_indices : [0])
  const selectedParts = new Set(selectedPartKeys())

  let appliedCount = 0
  gltfRoot.traverse((child) => {
    if (!(child instanceof THREE.Mesh || child instanceof THREE.SkinnedMesh)) return
    if (partControls.value.length > 0 && (!child.userData.partKey || !selectedParts.has(child.userData.partKey))) return
    const materials = Array.isArray(child.material) ? child.material : [child.material]
    materials.forEach((_, index) => {
      if (partControls.value.length > 0) {
        if (!selectedOverride) return
      } else {
      if (!selectedOverride || !selectedSubMeshes.has(index)) return
      }
      replaceMeshMaterialTexture(child, index, selectedOverride.png_path)
      appliedCount += 1
    })
  })

  if (appliedCount === 0 && selectedOverride && data.diffuse_texture_user_selected) {
    gltfRoot.traverse((child) => {
      if (!(child instanceof THREE.Mesh || child instanceof THREE.SkinnedMesh)) return
      if (partControls.value.length > 0 && (!child.userData.partKey || !selectedParts.has(child.userData.partKey))) return
      const materials = Array.isArray(child.material) ? child.material : [child.material]
      materials.forEach((_, index) => {
        if (partControls.value.length === 0) {
        if (!selectedSubMeshes.has(index)) return
        }
        replaceMeshMaterialTexture(child, index, selectedOverride.png_path)
        appliedCount += 1
      })
    })
  }
  requestRender()
}

function replaceMeshMaterialTexture(
  mesh: THREE.Mesh | THREE.SkinnedMesh,
  materialIndex: number,
  texturePath: string,
  flipY = false,
) {
  const currentMaterials = Array.isArray(mesh.material) ? mesh.material : [mesh.material]
  const sourceMaterial = currentMaterials[materialIndex] || currentMaterials[0]
  if (!sourceMaterial) return

  const nextMaterial = sourceMaterial.clone() as THREE.MeshStandardMaterial
  nextMaterial.userData = {
    ...nextMaterial.userData,
    assetFinderTextureOverride: true,
  }
  if (nextMaterial.map) nextMaterial.map.dispose()
  nextMaterial.map = createDiffuseTexture(texturePath, flipY)
  nextMaterial.color.set(0xffffff)
  nextMaterial.needsUpdate = true

  if (Array.isArray(mesh.material)) {
    const previousMaterial = mesh.material[materialIndex]
    const nextMaterials = [...mesh.material]
    nextMaterials[materialIndex] = nextMaterial
    mesh.material = nextMaterials
    disposeOverrideMaterial(previousMaterial)
  } else {
    const previousMaterial = mesh.material
    mesh.material = nextMaterial
    disposeOverrideMaterial(previousMaterial)
  }
}

function disposeOverrideMaterial(material: THREE.Material | undefined) {
  if (!material?.userData?.assetFinderTextureOverride) return
  disposeMaterialOnce(material, new Set())
}

function findSelectedTextureOverride(data: PreviewMaterialData) {
  if (!data.diffuse_texture_path) return null
  const candidates = data.texture_candidates || []
  const byPath = candidates.find(candidate => candidate.png_path === data.diffuse_texture_path)
  if (byPath) return byPath
  if (data.diffuse_texture_path_id || data.diffuse_texture_bundle_path) {
    const byIdentity = candidates.find(candidate =>
      (!data.diffuse_texture_path_id || candidate.texture_path_id === data.diffuse_texture_path_id)
      && (!data.diffuse_texture_bundle_path || candidate.texture_bundle_path === data.diffuse_texture_bundle_path)
      && (data.diffuse_material_index === undefined || candidate.material_index === data.diffuse_material_index)
      && (!data.diffuse_slot_name || candidate.slot_name === data.diffuse_slot_name)
    )
    if (byIdentity) return byIdentity
  }
  return candidates.find(candidate =>
    candidate.png_path === data.diffuse_texture_path
    && (!data.diffuse_material_name || candidate.material_name === data.diffuse_material_name)
    && (data.diffuse_material_index === undefined || candidate.material_index === data.diffuse_material_index)
    && (!data.diffuse_slot_name || candidate.slot_name === data.diffuse_slot_name)
  ) || null
}

function clearSelectionOverlays() {
  if (outlinePass) outlinePass.selectedObjects = []
  if (!scene) {
    highlightedSelectionOverlays = []
    return
  }
  for (const overlay of highlightedSelectionOverlays) {
    scene.remove(overlay)
    disposeOverlayObject(overlay)
  }
  highlightedSelectionOverlays = []
}

function updateSubMeshOverlay(data: {
  indices?: number[]
  sub_meshes?: Array<{ index_start: number; index_count: number }>
  selected_submesh_indices?: number[]
} | null) {
  if (!scene) return
  clearSelectionOverlays()
  const mesh = findPreviewMesh()
  if (!mesh || !data?.sub_meshes?.length) return
  const selectedIndices = data.selected_submesh_indices?.length ? data.selected_submesh_indices : [0]
  const visibleIndices = subMeshControls.value.length
    ? subMeshControls.value.filter(item => item.visible).map(item => item.index)
    : data.sub_meshes.map((_, index) => index)
  const lineSegments = buildSubMeshBoundaryOverlay(mesh, data, selectedIndices, visibleIndices, true)
  if (!lineSegments) return
  lineSegments.userData[PREVIEW_SUBMESH_OVERLAY_FLAG] = true
  highlightedSelectionOverlays.push(lineSegments)
  scene.add(lineSegments)
}

function updatePartSelectionOverlay() {
  if (!scene || hasSubMeshControls.value) return
  clearSelectionOverlays()
  const selectedKeys = new Set(selectedPartKeys())
  const selectedObjects: THREE.Object3D[] = []
  for (const mesh of findPreviewMeshes()) {
    const partKey = mesh.userData.partKey
    if (!partKey || !mesh.visible) continue
    if (selectedKeys.has(partKey)) selectedObjects.push(mesh)
  }
  if (outlinePass) outlinePass.selectedObjects = selectedObjects
}

function buildSubMeshBoundaryOverlay(
  mesh: THREE.Mesh,
  data: {
    indices?: number[]
    sub_meshes?: Array<{ index_start: number; index_count: number }>
  },
  selectedIndices: number[],
  visibleIndices: number[],
  includeUnselected = true,
) {
  const geometry = mesh.geometry
  const positionAttribute = geometry.getAttribute('position')
  if (!positionAttribute || !data.indices?.length || !data.sub_meshes || data.sub_meshes.length <= 1) return null

  const selectedSet = new Set(selectedIndices)
  const visibleSet = new Set(visibleIndices)
  const edgeMap = new Map<string, {
    a: number
    b: number
    subMeshes: Set<number>
    count: number
  }>()
  const addEdge = (a: number, b: number, subMeshIndex: number) => {
    const key = a < b ? `${a}:${b}` : `${b}:${a}`
    const entry = edgeMap.get(key)
    if (entry) {
      entry.subMeshes.add(subMeshIndex)
      entry.count += 1
      return
    }
    edgeMap.set(key, {
      a,
      b,
      subMeshes: new Set([subMeshIndex]),
      count: 1,
    })
  }

  for (let subMeshIndex = 0; subMeshIndex < data.sub_meshes.length; subMeshIndex++) {
    if (!visibleSet.has(subMeshIndex)) continue
    const subMesh = data.sub_meshes[subMeshIndex]
    const start = Math.max(0, subMesh.index_start)
    const end = Math.min(data.indices.length, start + subMesh.index_count)
    for (let i = start; i + 2 < end; i += 3) {
      const a = data.indices[i]
      const b = data.indices[i + 1]
      const c = data.indices[i + 2]
      addEdge(a, b, subMeshIndex)
      addEdge(b, c, subMeshIndex)
      addEdge(c, a, subMeshIndex)
    }
  }

  const vertices: number[] = []
  const colors: number[] = []
  const selectedColor = new THREE.Color(SUBMESH_OUTLINE_COLOR)
  const unselectedColor = new THREE.Color(SUBMESH_OUTLINE_COLOR)
  const pushEdge = (a: number, b: number, selected: boolean) => {
    vertices.push(
      positionAttribute.getX(a), positionAttribute.getY(a), positionAttribute.getZ(a),
      positionAttribute.getX(b), positionAttribute.getY(b), positionAttribute.getZ(b),
    )
    const color = selected ? selectedColor : unselectedColor
    colors.push(color.r, color.g, color.b, color.r, color.g, color.b)
  }

  for (const edge of edgeMap.values()) {
    const isBoundary = edge.count === 1 || edge.subMeshes.size > 1
    if (!isBoundary) continue
    const touchesSelected = Array.from(edge.subMeshes).some(subMeshIndex => selectedSet.has(subMeshIndex))
    const touchesVisible = Array.from(edge.subMeshes).some(subMeshIndex => visibleSet.has(subMeshIndex))
    if (!touchesVisible) continue
    if (!includeUnselected && !touchesSelected) continue
    pushEdge(edge.a, edge.b, touchesSelected)
  }

  if (vertices.length === 0) return null

  const overlayGeometry = new THREE.BufferGeometry()
  overlayGeometry.setAttribute('position', new THREE.Float32BufferAttribute(vertices, 3))
  overlayGeometry.setAttribute('color', new THREE.Float32BufferAttribute(colors, 3))
  const overlayMaterial = new THREE.LineBasicMaterial({
    transparent: true,
    opacity: 0.9,
    vertexColors: true,
    depthTest: false,
    depthWrite: false,
  })
  const lines = new THREE.LineSegments(overlayGeometry, overlayMaterial)
  lines.position.copy(mesh.position)
  lines.quaternion.copy(mesh.quaternion)
  lines.scale.copy(mesh.scale)
  lines.renderOrder = 20
  return lines
}

function disposeOverlayObject(overlay: THREE.Object3D) {
  overlay.traverse((child) => {
    if (child instanceof THREE.LineSegments || child instanceof THREE.Mesh) {
      child.geometry.dispose()
      if (Array.isArray(child.material)) {
        child.material.forEach(material => material.dispose())
      } else if (child.material) {
        child.material.dispose()
      }
    }
  })
}

function handleViewerPointerDown(event: PointerEvent) {
  if (event.button !== 0) {
    if (event.button === 1) {
      event.preventDefault()
      stopViewerInteractionRender()
    }
    return
  }
  if (!containerRef.value || !camera || !scene || !raycaster || !pointerNdc) return
  containerRef.value.focus()
  const meshes = findPreviewMeshes().filter(mesh => mesh.visible)
  if (!meshes.length) return
  const rect = containerRef.value.getBoundingClientRect()
  pointerNdc.x = ((event.clientX - rect.left) / rect.width) * 2 - 1
  pointerNdc.y = -((event.clientY - rect.top) / rect.height) * 2 + 1
  raycaster.setFromCamera(pointerNdc, camera)
  const intersections = raycaster.intersectObjects(meshes, true)
  if (!intersections.length) return
  const hit = findFirstVisibleIntersection(intersections)
  if (!hit) return
  const hitMesh = hit.object instanceof THREE.Mesh ? hit.object : null
  if (!hitMesh) return
  const hitPartKey = findPartKeyFromObject(hit.object)
  if (!hasSubMeshControls.value && hitPartKey) {
    selectSinglePart(hitPartKey)
    return
  }
  const faceIndex = hit.faceIndex ?? -1
  if (faceIndex < 0 || !activeSubMeshes.length) return
  const subMeshIndex = subMeshIndexForTriangle(faceIndex * 3)
  if (subMeshIndex < 0) return
  handleSubMeshListClick(subMeshIndex)
}

function findPartKeyFromObject(object: THREE.Object3D | null) {
  let current: THREE.Object3D | null = object
  while (current) {
    const partKey = current.userData?.partKey
    if (typeof partKey === 'string' && partKey) return partKey
    current = current.parent
  }
  return ''
}

function handleViewerKeyDown(event: KeyboardEvent) {
  const key = event.key.toLowerCase()
  if ((event as KeyboardEvent & { __assetFinderHandled?: boolean }).__assetFinderHandled) return
  if (loadState.value !== 'ready') return
  const target = event.target as HTMLElement | null
  if (target && ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName)) return
  if (['w', 'a', 's', 'd', 'q', 'e'].includes(key) && isViewerKeyboardTarget(target)) {
    event.preventDefault()
    ;(event as KeyboardEvent & { __assetFinderHandled?: boolean }).__assetFinderHandled = true
    activeMovementKeys.add(key)
    startKeyboardCameraMove()
    return
  }
  if (key !== 'h') return
  if (!hasMeshRegionControls.value) return
  event.preventDefault()
  ;(event as KeyboardEvent & { __assetFinderHandled?: boolean }).__assetFinderHandled = true
  toggleSelectedMeshRegionVisibility()
}

function handleViewerKeyUp(event: KeyboardEvent) {
  const key = event.key.toLowerCase()
  if (!activeMovementKeys.delete(key)) return
  if (activeMovementKeys.size === 0) {
    stopKeyboardCameraMove()
  }
}

function isViewerKeyboardTarget(target: HTMLElement | null) {
  return Boolean(containerRef.value && target && containerRef.value.contains(target))
}

function startKeyboardCameraMove() {
  if (keyboardMoveRenderId !== null) return
  lastKeyboardMoveTime = performance.now()
  const tick = () => {
    keyboardMoveRenderId = null
    const now = performance.now()
    const delta = Math.min(0.05, (now - lastKeyboardMoveTime) / 1000)
    lastKeyboardMoveTime = now
    moveCameraFromKeyboard(delta)
    if (activeMovementKeys.size > 0) {
      keyboardMoveRenderId = requestAnimationFrame(tick)
    }
  }
  keyboardMoveRenderId = requestAnimationFrame(tick)
}

function stopKeyboardCameraMove() {
  if (keyboardMoveRenderId !== null) {
    cancelAnimationFrame(keyboardMoveRenderId)
    keyboardMoveRenderId = null
  }
  requestRender()
}

function moveCameraFromKeyboard(deltaSeconds: number) {
  if (!camera || !controls) return
  const forward = new THREE.Vector3()
  camera.getWorldDirection(forward)
  forward.normalize()
  const right = new THREE.Vector3().crossVectors(forward, camera.up).normalize()
  const up = new THREE.Vector3(0, 1, 0)
  const move = new THREE.Vector3()
  if (activeMovementKeys.has('w')) move.add(forward)
  if (activeMovementKeys.has('s')) move.sub(forward)
  if (activeMovementKeys.has('d')) move.add(right)
  if (activeMovementKeys.has('a')) move.sub(right)
  if (activeMovementKeys.has('e')) move.add(up)
  if (activeMovementKeys.has('q')) move.sub(up)
  if (move.lengthSq() <= 0) return
  move.normalize().multiplyScalar(cameraKeyboardMoveSpeed() * deltaSeconds)
  camera.position.add(move)
  controls.target.add(move)
  controls.update()
  renderFrame()
}

function cameraKeyboardMoveSpeed() {
  if (!camera || !controls) return 3
  return Math.max(0.25, camera.position.distanceTo(controls.target) * 1.8)
}

function findFirstVisibleIntersection(intersections: THREE.Intersection[]) {
  if (!hasSubMeshControls.value || !activeSubMeshes.length) {
    return intersections[0] || null
  }
  const visibleSet = new Set(subMeshControls.value.filter(item => item.visible).map(item => item.index))
  for (const hit of intersections) {
    const faceIndex = hit.faceIndex ?? -1
    if (faceIndex < 0) continue
    const subMeshIndex = subMeshIndexForTriangle(faceIndex * 3)
    if (subMeshIndex >= 0 && visibleSet.has(subMeshIndex)) return hit
  }
  return null
}

function subMeshIndexForTriangle(triangleIndex: number) {
  return activeSubMeshes.findIndex(subMesh =>
    triangleIndex >= subMesh.index_start && triangleIndex < subMesh.index_start + subMesh.index_count,
  )
}

function findPreviewMesh(): THREE.Mesh | null {
  return findPreviewMeshes()[0] || null
}

function findPreviewMeshes(): THREE.Mesh[] {
  if (!scene) return []
  const result: THREE.Mesh[] = []
  scene.traverse((child) => {
    if (child instanceof THREE.Mesh && child.userData[PREVIEW_MESH_FLAG]) {
      result.push(child)
    }
  })
  return result
}

function createPreviewMaterial(texturePath?: string, flipY = false) {
  const diffuseTexture = createDiffuseTexture(texturePath, flipY)
  const material = new THREE.MeshStandardMaterial({
    color: diffuseTexture ? 0xffffff : PREVIEW_DEFAULT_COLORS.meshFallback,
    map: diffuseTexture || null,
    metalness: 0.05,
    roughness: 0.3,
    envMapIntensity: 0.7,
    side: THREE.DoubleSide,
  })
  return material
}

function visibleSubMeshIndices(data: { sub_meshes?: Array<unknown> }) {
  if (subMeshControls.value.length) {
    return new Set(subMeshControls.value.filter(item => item.visible).map(item => item.index))
  }
  return new Set((data.sub_meshes || []).map((_, index) => index))
}

function applySubMeshGroups(
  geometry: THREE.BufferGeometry,
  data: {
    indices: number[]
    sub_meshes?: Array<{ index_start: number; index_count: number; topology?: number }>
  },
  materialCount: number,
  visibleIndices = visibleSubMeshIndices(data),
) {
  if (!data.sub_meshes?.length || !data.indices?.length || materialCount <= 1) return
  geometry.clearGroups()
  for (let materialIndex = 0; materialIndex < data.sub_meshes.length; materialIndex++) {
    if (!visibleIndices.has(materialIndex)) continue
    const subMesh = data.sub_meshes[materialIndex]
    const start = Math.max(0, subMesh.index_start)
    const count = Math.max(0, Math.min(subMesh.index_count, data.indices.length - start))
    if (count > 0) {
      geometry.addGroup(start, count, Math.min(materialIndex, materialCount - 1))
    }
  }
}

function buildUvBuffer(uvs: number[], vertexCount: number) {
  return new Float32Array(uvs.slice(0, vertexCount * 2))
}

function createDiffuseTexture(path?: string, flipY = false): THREE.Texture | null {
  if (!path) return null
  if (!textureLoader) textureLoader = new THREE.TextureLoader()
  const texture = textureLoader.load(path, () => requestRender())
  texture.colorSpace = THREE.SRGBColorSpace
  texture.wrapS = THREE.RepeatWrapping
  texture.wrapT = THREE.RepeatWrapping
  texture.flipY = flipY
  return texture
}

function buildCloudBackground(): THREE.Color | THREE.Texture {
  const canvas = document.createElement('canvas')
  canvas.width = 512
  canvas.height = 320
  const ctx = canvas.getContext('2d')
  if (!ctx) return new THREE.Color(PREVIEW_DEFAULT_COLORS.backgroundFallback)

  const gradient = ctx.createLinearGradient(0, 0, 0, canvas.height)
  gradient.addColorStop(0, '#cfd8e6')
  gradient.addColorStop(0.55, '#93a8bf')
  gradient.addColorStop(1, '#566579')
  ctx.fillStyle = gradient
  ctx.fillRect(0, 0, canvas.width, canvas.height)

  const paintCloud = (x: number, y: number, scale: number, alpha: number) => {
    const cloud = ctx.createRadialGradient(x, y, 8 * scale, x, y, 70 * scale)
    cloud.addColorStop(0, `rgba(255,255,255,${alpha})`)
    cloud.addColorStop(0.5, `rgba(255,255,255,${alpha * 0.55})`)
    cloud.addColorStop(1, 'rgba(255,255,255,0)')
    ctx.fillStyle = cloud
    ctx.beginPath()
    ctx.arc(x, y, 70 * scale, 0, Math.PI * 2)
    ctx.fill()
  }

  paintCloud(95, 70, 1.2, 0.24)
  paintCloud(185, 48, 1.5, 0.22)
  paintCloud(310, 88, 1.35, 0.2)
  paintCloud(425, 62, 1.1, 0.18)
  paintCloud(135, 180, 1.0, 0.14)
  paintCloud(320, 190, 1.4, 0.16)

  const tex = new THREE.CanvasTexture(canvas)
  tex.colorSpace = THREE.SRGBColorSpace
  tex.mapping = THREE.EquirectangularReflectionMapping
  cloudTexture = tex
  return tex
}

/** Clear models from scene */
function clearModel() {
  if (!scene) return
  stopAnimationPlayback()
  gltfRoot = null
  activeGltf = null
  // Remove all models (keep lights and grid)
  const toRemove: THREE.Object3D[] = []
  const disposedGeometries = new Set<THREE.BufferGeometry>()
  const disposedMaterials = new Set<THREE.Material>()
  scene.traverse((child) => {
    if (
      child.type === 'Mesh' ||
      child.type === 'Group' ||
      child.type === 'SkinnedMesh'
    ) {
      toRemove.push(child)
    }
  })
  for (const child of toRemove) {
    scene.remove(child)
    // Dispose geometry and materials to reduce memory leaks
    if (child instanceof THREE.Mesh) {
      if (!disposedGeometries.has(child.geometry)) {
        child.geometry.dispose()
        disposedGeometries.add(child.geometry)
      }
      if (Array.isArray(child.material)) {
        child.material.forEach((m) => disposeMaterialOnce(m, disposedMaterials))
      } else if (child.material) {
        disposeMaterialOnce(child.material, disposedMaterials)
      }
    }
  }
  clearSelectionOverlays()
}

function disposeMaterials(material: THREE.Material | THREE.Material[]) {
  const disposed = new Set<THREE.Material>()
  if (Array.isArray(material)) {
    material.forEach((item) => disposeMaterialOnce(item, disposed))
  } else {
    disposeMaterialOnce(material, disposed)
  }
}

function disposeMaterialOnce(material: THREE.Material, disposed: Set<THREE.Material>) {
  if (disposed.has(material)) return
  const maybeMapped = material as THREE.Material & { map?: THREE.Texture | null }
  if (maybeMapped.map) {
    maybeMapped.map.dispose()
    maybeMapped.map = null
  }
  material.dispose()
  disposed.add(material)
}

// ============================================================
// Lifecycle
// ============================================================

onMounted(() => {
  window.addEventListener('keydown', handleViewerKeyDown)
  window.addEventListener('keyup', handleViewerKeyUp)
  window.addEventListener('pointerup', handleGlobalPointerEnd, true)
  window.addEventListener('mouseup', handleGlobalPointerEnd, true)
  window.addEventListener('pointercancel', handleGlobalInteractionCancel, true)
  window.addEventListener('blur', handleGlobalInteractionCancel)
  if (containerRef.value) {
    // Step 1: Initialize Three.js scene, create renderer and camera
    initScene(containerRef.value)

    // Step 2: Set up ResizeObserver to listen for container size changes.
    // When the user resizes the window causing the preview area dimensions to change,
    // automatically update renderer.setSize and camera.aspect,
    // ensuring 3D content always matches the container size without display issues or aspect ratio problems.
    resizeObserver = new ResizeObserver((entries) => {
      for (const entry of entries) {
        // entry.target is the observed container DOM element, i.e., containerRef
        handleContainerResize(entry.target as HTMLElement)
      }
    })
    resizeObserver.observe(containerRef.value)

    // If geometry data is already present, render immediately
    if (props.geometryData && scene) {
      buildMeshFromData(props.geometryData)
    }
  }
})

onUnmounted(() => {
  window.removeEventListener('keydown', handleViewerKeyDown)
  window.removeEventListener('keyup', handleViewerKeyUp)
  window.removeEventListener('pointerup', handleGlobalPointerEnd, true)
  window.removeEventListener('mouseup', handleGlobalPointerEnd, true)
  window.removeEventListener('pointercancel', handleGlobalInteractionCancel, true)
  window.removeEventListener('blur', handleGlobalInteractionCancel)
  // Disconnect ResizeObserver to prevent memory leaks after component unmount
  if (resizeObserver) {
    resizeObserver.disconnect()
    resizeObserver = null
  }
  disposeScene()
})

// Watch geometry data changes (re-render when switching models)
watch(
  () => [
    props.geometryData?.vertices,
    props.geometryData?.indices,
    props.geometryData?.normals,
    props.geometryData?.uvs,
    props.geometryData?.sub_meshes,
    props.geometryData?.parts,
    props.geometryData?.glb_url,
    props.geometryData?.glb_path,
  ] as const,
  (next, previous) => {
    if (previous && next.every((value, index) => value === previous[index])) {
      return
    }
    if (props.geometryData && scene) {
      buildMeshFromData(props.geometryData)
    }
  },
)

watch(
  () => props.geometryData?.diffuse_texture_path,
  () => {
    if (props.geometryData && scene && loadState.value === 'ready') {
      updatePreviewMaterials(props.geometryData)
      updateSubMeshOverlay(props.geometryData)
    }
  },
)

</script>

<template>
  <div
    class="model-viewer"
    :class="{ 'has-side-panel': false }"
  >
    <div
      class="model-stage"
      ref="containerRef"
      tabindex="0"
      @auxclick.prevent
      @contextmenu.prevent
      @pointercancel="handleGlobalInteractionCancel"
      @pointerdown="handleViewerPointerDown"
    >
      <!-- Loading -->
      <div v-if="loadState === 'loading'" class="viewer-overlay">
        <el-icon class="is-loading" :size="32"><Refresh /></el-icon>
        <p>{{ t('assetPreview.loadingModel') }}</p>
      </div>

      <!-- Load failed -->
      <div v-else-if="loadState === 'error'" class="viewer-overlay viewer-error">
        <p>{{ t('assetPreview.modelLoadFailed') }}</p>
        <p class="error-detail">{{ loadError }}</p>
      </div>
    </div>

  </div>
</template>

<style scoped>
.model-viewer {
  width: 100%;
  height: 100%;
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  overflow: hidden;
  background: #2f2f2f;
  align-self: stretch;
  min-height: 0;
}

.model-viewer.has-side-panel {
  grid-template-columns: minmax(0, 1fr) minmax(240px, 300px);
}

.model-stage {
  position: relative;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  background: #2f2f2f;
}

.model-stage:focus {
  outline: none;
}

.model-stage :deep(canvas) {
  display: block;
  width: 100%;
  height: 100%;
  user-select: none;
  touch-action: none;
}

.viewer-overlay {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: rgba(232, 236, 242, 0.72);
  font-size: 13px;
  z-index: 1;
  pointer-events: none;
}

.viewer-error {
  color: #d34947;
}

.error-detail {
  font-size: 11px;
  max-width: 90%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.submesh-panel-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 8px;
  overflow: auto;
}

.submesh-row {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  min-height: 28px;
  padding: 6px 8px;
  border: 1px solid rgba(255, 255, 255, 0.14);
  border-radius: 6px;
  color: rgba(255, 255, 255, 0.86);
  background: rgba(255, 255, 255, 0.06);
  cursor: pointer;
  text-align: left;
}

.submesh-row:hover,
.submesh-row.selected {
  border-color: rgba(255, 225, 90, 0.78);
  background: rgba(255, 225, 90, 0.12);
}

.submesh-row.is-hidden {
  opacity: 0.52;
}

.submesh-row input {
  margin: 0;
}

.submesh-row-swatch {
  width: 10px;
  height: 10px;
  border-radius: 999px;
  flex: 0 0 auto;
}

.submesh-row-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
}

.viewer-hint {
  font-size: 11px;
  opacity: 0.6;
}

.model-side-panel {
  display: flex;
  flex-direction: column;
  gap: 0;
  min-width: 0;
  min-height: 0;
  border-left: 1px solid rgba(255, 255, 255, 0.14);
  background: rgba(24, 28, 34, 0.96);
}

.side-section {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  flex: 1 1 0;
  padding: 10px;
  overflow: hidden;
}

.side-section + .side-section {
  border-top: 1px solid rgba(255, 255, 255, 0.12);
}

.side-section-title {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  color: rgba(255, 255, 255, 0.92);
  font-size: 12px;
  font-weight: 700;
  line-height: 1.2;
}

.side-section-subtitle {
  color: rgba(255, 255, 255, 0.52);
  font-size: 11px;
  font-weight: 500;
}

.texture-panel-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
  margin-top: 8px;
  overflow: auto;
}

.texture-row {
  display: grid;
  grid-template-columns: 48px minmax(0, 1fr);
  gap: 8px;
  width: 100%;
  min-height: 60px;
  padding: 6px;
  border: 1px solid rgba(255, 255, 255, 0.14);
  border-radius: 6px;
  color: rgba(255, 255, 255, 0.88);
  background: rgba(255, 255, 255, 0.06);
  cursor: pointer;
  text-align: left;
}

.texture-row:hover,
.texture-row.was-applied {
  border-color: rgba(255, 225, 90, 0.78);
  background: rgba(255, 225, 90, 0.12);
}

.texture-row-image {
  display: block;
  width: 48px;
  height: 48px;
  object-fit: cover;
  border-radius: 4px;
  background: rgba(255, 255, 255, 0.08);
}

.texture-row-body {
  display: flex;
  min-width: 0;
  flex-direction: column;
  justify-content: center;
  gap: 2px;
}

.texture-row-name,
.texture-row-meta {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.texture-row-name {
  font-size: 12px;
  font-weight: 650;
}

.texture-row-meta {
  color: rgba(255, 255, 255, 0.56);
  font-size: 11px;
}

:global(.mesh-texture-tooltip-content) {
  display: flex;
  flex-direction: column;
  gap: 3px;
  max-width: 280px;
  white-space: pre-line;
}

@media (max-width: 860px) {
  .model-viewer {
    grid-template-columns: 1fr;
    grid-template-rows: minmax(0, 1fr) minmax(180px, 34%);
  }

  .model-viewer.has-side-panel {
    grid-template-columns: 1fr;
  }

  .model-side-panel {
    flex-direction: row;
    border-top: 1px solid rgba(255, 255, 255, 0.14);
    border-left: 0;
  }

  .side-section + .side-section {
    border-top: 0;
    border-left: 1px solid rgba(255, 255, 255, 0.12);
  }
}
</style>
