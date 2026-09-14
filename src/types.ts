/**
 * types.ts - All type definitions, constants and icon mappings for the WorkSpace page
 *
 * Centralizing types avoids circular dependencies and makes it easier for the Rust side to understand the interface contract.
 */

// ============================================================
// Bundle file entry (corresponds to Rust workspace.rs)
// ============================================================

export interface BundleFileEntry {
  name: string
  path: string
  size: number
  modified_ms: number
  modified: string
}

// ============================================================
// Bundle node info
// ============================================================

export interface BundleNodeInfo {
  name: string
  offset: number
  size: number
}

// ============================================================
// Asset summary (corresponds to Rust workspace.rs)
// ============================================================

export interface AssetSummary {
  path: string
  /** Asset instance name (m_Name), from the Unity object's Name field */
  name: string
  class_name: string
  class_id: number
  /** i64 serialized as string to avoid JS Number precision loss */
  path_id: string
  byte_size: number
  /** Frontend cache origin, used by aggregated views that merge multiple bundles. */
  source_bundle_path?: string
}

// ============================================================
// Bundle metadata (full parsed info)
// ============================================================

export interface BundleMeta {
  path: string
  size: number
  compressed: boolean
  unity_version: string
  nodes: BundleNodeInfo[]
  assets: AssetSummary[]
  files_diag: FileDiag[]
  parse_logs: string[]
}

// ============================================================
// AssetMap index query types
// ============================================================

export interface MapSummary {
  built_at: number
  built_at_formatted: string
  bundle_count: number
  asset_count: number
  parsed_count: number
  cancelled: boolean
}

export interface MapAssetClassStat {
  name: string
  count: number
}

export interface AssetSearchIndexStatus {
  ready: boolean
  asset_count: number
  indexed_count: number
  indexed_asset_count?: number | null
}

export interface MapAssetQueryOptions {
  search: string
  class_names: string[]
  bundle_path?: string
  name_query?: string
  name_match_mode?: 'contains' | 'starts_with' | 'ends_with' | 'equals'
  name_include_queries?: string[]
  name_exclude_query?: string
  name_exclude_queries?: string[]
  bundle_query?: string
  path_id_query?: string
  min_size?: number | null
  max_size?: number | null
  offset: number
  limit: number
  sort_by?: 'name' | 'type' | 'size' | 'bundle' | 'path_id'
  sort_direction?: 'asc' | 'desc'
}

export interface MapAssetQueryResult {
  assets: AssetSummary[]
  total: number
  has_more?: boolean
  offset: number
  limit: number
}

export type AdvancedAssetMatchMode = 'contains' | 'starts_with' | 'ends_with' | 'equals'
export type AdvancedAssetSizeUnit = 'B' | 'KB' | 'MB'
export type AdvancedAssetSortField = 'type' | 'name' | 'size' | 'bundle' | 'path_id'
export type AdvancedAssetSortDirection = 'asc' | 'desc'

export interface AdvancedAssetFilterState {
  classNames: string[]
  nameQuery: string
  nameMatchMode: AdvancedAssetMatchMode
  nameIncludeQueries: string[]
  nameExcludeQuery: string
  nameExcludeQueries: string[]
  bundleQuery: string
  pathIdQuery: string
  minSizeText: string
  maxSizeText: string
  sizeUnit: AdvancedAssetSizeUnit
  sortBy: AdvancedAssetSortField
  sortDirection: AdvancedAssetSortDirection
}

// ============================================================
// Scene Hierarchy
// ============================================================

/// Scene hierarchy tree node
export interface SceneNode {
  name: string
  path_id: string
  class_id: number
  children: SceneNode[]
  components: ComponentRef[]
}

/// Component reference summary
export interface ComponentRef {
  path_id: string
  class_name: string
  class_id: number
}

/// Scene hierarchy tree + diagnostic info
export interface SceneHierarchyResult {
  roots: SceneNode[]
  diagnostics: string[]
}

// ============================================================
// Dump result (corresponds to Rust dump_commands.rs)
// ============================================================

/// Asset TypeTree dump result
export interface DumpResult {
  dump_text: string
  truncated: boolean
  truncated_items: number
  has_typetree: boolean
}

export interface AssetTypedPreviewResult {
  class_name: string
  path_id: string
  name: string
  unity_version: string
  byte_size: number
  sections: AssetPreviewSection[]
  relations: AssetPreviewRelation[]
  warnings: string[]
}

export interface AssetPreviewSection {
  title: string
  kind: string
  rows: AssetPreviewRow[]
}

export interface AssetPreviewRow {
  label: string
  value: string
}

export interface AssetPreviewRelation {
  relation_type: string
  field_path: string
  direction: 'in' | 'out' | string
  bundle_path: string
  path_id: string
  class_name: string
  name: string
}

export interface PreviewAnimationClipRef {
  bundle_path: string
  path_id: string
  class_name: string
  name: string
  byte_size: number
}

export interface PreviewAnimatorRef {
  bundle_path: string
  path_id: string
  class_name: string
  name: string
  byte_size: number
}

/// Bundle parse result returned asynchronously via Channel
export interface BundleParseChannelResult {
  success: boolean
  meta: BundleMeta | null
  error: string | null
}

/// Parse diagnostics for a single file within a bundle
export interface FileDiag {
  name: string
  size_bytes: number
  first_bytes_hex: string
  parse_ok: boolean
  parse_error: string
}

// ============================================================
// Log entry
// ============================================================

export interface LogEntry {
  time: string
  message: string
  type: 'info' | 'success' | 'warn' | 'error'
}

// ============================================================
// Task system types (corresponds to Rust task_types.rs)
// ============================================================

/**
 * Backend task log entry.
 * Pushed from the Rust backend to the frontend log panel via persistent Channel.
 * level is serialized as a lowercase string, one-to-one mapping with LogEntry.type.
 */
export interface TaskLogEntry {
  /** Task ID that produced this log */
  task_id: string
  /** Task type label */
  task_label: string
  /** Log level */
  level: 'info' | 'success' | 'warn' | 'error'
  /** Log message content */
  message: string
  /** Unix timestamp (milliseconds) */
  timestamp_ms: number
  /** Progress: current step description */
  progress_step: string | null
  /** Progress: current completion count */
  progress_current: number | null
  /** Progress: total count */
  progress_total: number | null
}

/**
 * Task progress snapshot (for progress bar display).
 * Extracted from TaskLogEntry, deduplicated by task_id keeping the latest value.
 */
export interface TaskProgress {
  task_id: string
  task_label: string
  step: string
  current: number
  total: number
}

// ============================================================
// Decrypt game selection
// ============================================================

export enum DecryptGameName {
  GirlsFrontline2 = 'Girls Frontline2',
  NarakaBladepoint = 'Naraka Bladepoint',
  TheMagicBlade = 'The Magic Blade',
}

// ============================================================
// Type scan cache entry (for MD5 incremental cache)
// ============================================================

export interface TypeCacheEntry {
  md5: string
  types: string[]
}

// ============================================================
// Complete list of Unity asset types
//
// Similar to AssetStudio, shows all known types for user filter selection.
// Statically defined, not sent to the Rust side.
// ============================================================

export const ALL_ASSET_TYPES: string[] = [
  'Mesh', 'Texture2D', 'Sprite', 'AudioClip', 'TextAsset',
  'MonoBehaviour', 'Shader', 'Material', 'AnimationClip',
  'Animator', 'GameObject', 'Transform', 'SkinnedMeshRenderer',
  'Renderer', 'MeshFilter', 'Rigidbody', 'Collider',
  'Light', 'Camera', 'ParticleSystem', 'VideoClip',
  'MovieTexture', 'Font', 'TerrainData', 'Avatar',
  'RuntimeAnimatorController', 'BlendTree',
]

// Class name to icon mapping has been migrated to src/utils/ClassIconUtils.ts

// ============================================================
// Export types (corresponds to Rust export_types.rs)
// ============================================================

export type ExportFormat =
  | 'dds' | 'png' | 'tga'
  | 'glb' | 'obj'
  | 'wav'
  | 'txt' | 'json' | 'bytes'
  | 'ttf' | 'otf'
  | 'shader' | 'bin'
  | 'jsontree'
  | 'ogv' | 'binraw'
  | 'raw'

export type ExportGroupBy = 'none' | 'bytype' | 'bycontainer'

export interface ExportOptions {
  output_dir: string
  group_by: ExportGroupBy
  overwrite_existing: boolean
  generate_report: boolean
  include_materials: boolean
  include_textures: boolean
  include_skeleton: boolean
  include_animations: boolean
  split_submeshes: boolean
  workspace_dirs: string[]
  asset_map_cache_root?: string | null
  format_overrides: Record<string, ExportFormat>
}

export interface ExportAssetRef {
  bundle_path: string
  path_id: string
  class_name: string
  asset_name: string
}

export interface ExportJob {
  job_id: string
  assets: ExportAssetRef[]
  options: ExportOptions
}

export interface ExportResult {
  asset_name: string
  class_name: string
  path_id: string
  output_file: string
  format: ExportFormat
  success: boolean
  error: string | null
  byte_size: number
  duration_ms: number
}

export interface ExportProgress {
  job_id: string
  done: number
  total: number
  current_asset: string
  current_step: string
  completed: ExportResult[]
  cancelled: boolean
}

export interface ExportReport {
  job_id: string
  total: number
  succeeded: number
  failed: number
  results: ExportResult[]
  output_directory: string
  duration_ms: number
}
