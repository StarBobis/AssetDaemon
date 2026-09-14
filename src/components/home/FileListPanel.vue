<script setup lang="ts">
/**
 * FileListPanel ->Upper-left section: .bundle file list
 *
 * Features:
 *   - Display all Unity file candidates in the current directory
 *   - Type filter popover (highlight matching files by asset type)
 *   - Keyboard up/down navigation through currently visible file nodes
 *   - Highlight currently selected file, highlight filter-matching files
 *   - Context menu: reveal file in file manager
 */

import { computed, nextTick, onMounted, onUnmounted, reactive, ref } from 'vue'
import { ArrowDown, ArrowRight, Folder, Refresh, Search } from '@element-plus/icons-vue'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import { useI18n } from 'vue-i18n'
import { useWorkspace } from '../../composables/home/useWorkspace'
import { ClassIconUtils } from '../../utils/ClassIconUtils'
import { FormatUtils } from '../../utils/FormatUtils'
import type { BundleFileEntry } from '../../types'
import CenteredFilterModal from './CenteredFilterModal.vue'

const ws = useWorkspace()
const { t } = useI18n()
const filterPopoverVisible = ref(false)
let unregisterFileKeyboardNavigator: (() => void) | null = null

function setFileListElement(element: unknown) {
  ws.fileListRef.value = element instanceof HTMLElement ? element : null
}

type FileMatchMode = 'contains' | 'starts_with' | 'ends_with' | 'equals'
type FileSizeUnit = 'B' | 'KB' | 'MB'
type FileSortField = 'path' | 'name' | 'size' | 'modified'
type FileSortDirection = 'asc' | 'desc'

interface FileFilterState {
  typeNames: string[]
  includeQueries: string[]
  excludeQueries: string[]
  matchMode: FileMatchMode
  minSizeText: string
  maxSizeText: string
  sizeUnit: FileSizeUnit
  sortBy: FileSortField
  sortDirection: FileSortDirection
}

type FileTreeNode = FileTreeDirectoryNode | FileTreeFileNode

interface FileTreeDirectoryNode {
  kind: 'dir'
  id: string
  name: string
  relativePath: string
  depth: number
  children: FileTreeNode[]
  fileCount: number
  totalSize: number
}

interface FileTreeFileNode {
  kind: 'file'
  id: string
  name: string
  depth: number
  file: BundleFileEntry
}

const expandedDirectories = ref<Set<string>>(new Set())

function createDefaultFileFilter(): FileFilterState {
  return {
    typeNames: [],
    includeQueries: [],
    excludeQueries: [],
    matchMode: 'contains',
    minSizeText: '',
    maxSizeText: '',
    sizeUnit: 'MB',
    sortBy: 'path',
    sortDirection: 'asc',
  }
}

const appliedFileFilter = ref<FileFilterState>(createDefaultFileFilter())
const draftFileFilter = reactive<FileFilterState>(createDefaultFileFilter())

function normalizeFileFilter(value: unknown): FileFilterState {
  const base = createDefaultFileFilter()
  if (!value || typeof value !== 'object') return base
  const filter = value as Partial<FileFilterState>
  return {
    ...base,
    ...filter,
    typeNames: normalizeTextList(filter.typeNames || []),
    includeQueries: normalizeTextList(filter.includeQueries || []),
    excludeQueries: normalizeTextList(filter.excludeQueries || []),
    matchMode: ['contains', 'starts_with', 'ends_with', 'equals'].includes(filter.matchMode || '')
      ? filter.matchMode as FileMatchMode
      : base.matchMode,
    sizeUnit: ['B', 'KB', 'MB'].includes(filter.sizeUnit || '')
      ? filter.sizeUnit as FileSizeUnit
      : base.sizeUnit,
    sortBy: ['path', 'name', 'size', 'modified'].includes(filter.sortBy || '')
      ? filter.sortBy as FileSortField
      : base.sortBy,
    sortDirection: filter.sortDirection === 'desc' ? 'desc' : 'asc',
  }
}

async function persistFileFilter() {
  const store = await ws.getStore()
  await store.set('file-list-advanced-filter', appliedFileFilter.value)
  await store.save()
}

async function persistExpandedDirectories() {
  const store = await ws.getStore()
  await store.set('file-list-expanded-directories', [...expandedDirectories.value])
  await store.save()
}

onMounted(async () => {
  unregisterFileKeyboardNavigator = ws.registerKeyboardNavigator('files', handleFileTreeKeydown)

  const store = await ws.getStore()
  const savedFilter = normalizeFileFilter(await store.get('file-list-advanced-filter'))
  appliedFileFilter.value = savedFilter
  Object.assign(draftFileFilter, savedFilter)
  ws.setActiveFilters(savedFilter.typeNames, { allowScanFallback: false })

  const savedExpanded = await store.get('file-list-expanded-directories')
  if (Array.isArray(savedExpanded)) {
    expandedDirectories.value = new Set(savedExpanded.filter((value): value is string => typeof value === 'string'))
  }
})

onUnmounted(() => {
  unregisterFileKeyboardNavigator?.()
  unregisterFileKeyboardNavigator = null
})

const fileFilterActiveCount = computed(() => {
  const filter = appliedFileFilter.value
  let count = filter.typeNames.length
  count += normalizeTextList(filter.includeQueries).length
  count += normalizeTextList(filter.excludeQueries).length
  if (filter.minSizeText.trim()) count += 1
  if (filter.maxSizeText.trim()) count += 1
  if (filter.sortBy !== 'path' || filter.sortDirection !== 'asc') count += 1
  return count
})

// ============================================================
// Context menu state
// ============================================================

/** Whether context menu is visible */
const contextMenuVisible = ref(false)

/** Context menu DOM, used to keep the menu inside the viewport */
const contextMenuRef = ref<HTMLElement | null>(null)

/** Context menu position */
const contextMenuPos = ref({ x: 0, y: 0 })

/** File that was right-clicked */
const contextMenuFile = ref<BundleFileEntry | null>(null)

/**
 * Show the context menu.
 *
 * Triggered when right-clicking a file list item.
 * Records the click position and corresponding file info,
 * then displays the custom context menu.
 */
function showContextMenu(event: MouseEvent, file: BundleFileEntry) {
  event.preventDefault()
  event.stopPropagation()
  contextMenuFile.value = file
  contextMenuPos.value = { x: event.clientX, y: event.clientY }
  contextMenuVisible.value = true
  nextTick(clampContextMenuPosition)
}

/**
 * Hide the context menu.
 *
 * Triggered when clicking a menu item, clicking blank area, or pressing Esc.
 */
function hideContextMenu() {
  contextMenuVisible.value = false
  contextMenuFile.value = null
}

function clampContextMenuPosition() {
  const menu = contextMenuRef.value
  if (!menu || !contextMenuVisible.value) return

  const gap = 8
  const maxX = window.innerWidth - menu.offsetWidth - gap
  const maxY = window.innerHeight - menu.offsetHeight - gap

  contextMenuPos.value = {
    x: Math.max(gap, Math.min(contextMenuPos.value.x, maxX)),
    y: Math.max(gap, Math.min(contextMenuPos.value.y, maxY)),
  }
}

function getMatchLabel(filePath: string) {
  const matches = ws.getFileMatchedFilters(filePath)
  if (matches.length === 0) return ''
  if (matches.length === 1) return matches[0]
  return t('common.matches', { count: matches.length })
}

function normalizePathSeparators(path: string): string {
  return path.replace(/\\/g, '/')
}

function getRelativeFilePath(file: BundleFileEntry): string {
  const normalizedFilePath = normalizePathSeparators(file.path)
  const normalizedWorkDir = normalizePathSeparators(ws.workDir.value).replace(/\/+$/, '')
  const lowerFilePath = normalizedFilePath.toLocaleLowerCase()
  const lowerWorkDir = normalizedWorkDir.toLocaleLowerCase()

  if (normalizedWorkDir && lowerFilePath === lowerWorkDir) {
    return file.name
  }
  if (normalizedWorkDir && lowerFilePath.startsWith(`${lowerWorkDir}/`)) {
    return normalizedFilePath.slice(normalizedWorkDir.length + 1)
  }
  return file.name || normalizedFilePath.split('/').pop() || normalizedFilePath
}

const fileTypeStats = computed(() => {
  const counts = new Map<string, number>()
  for (const file of ws.bundleFiles.value) {
    const types = ws.fileTypeMap.value[file.path]
    if (!types) continue
    for (const typeName of types) {
      counts.set(typeName, (counts.get(typeName) || 0) + 1)
    }
  }
  return ws.allClassNames.value
    .map(name => ({ name, count: counts.get(name) || 0 }))
    .sort((left, right) => {
      if (right.count !== left.count) return right.count - left.count
      return left.name.localeCompare(right.name)
    })
})

const popularFileTypes = computed(() => {
  const withCounts = fileTypeStats.value.filter(stat => stat.count > 0).slice(0, 12)
  return withCounts.length > 0 ? withCounts : fileTypeStats.value.slice(0, 12)
})

function syncDraftFileFilter() {
  Object.assign(draftFileFilter, {
    ...appliedFileFilter.value,
    typeNames: [...appliedFileFilter.value.typeNames],
    includeQueries: [...appliedFileFilter.value.includeQueries],
    excludeQueries: [...appliedFileFilter.value.excludeQueries],
  })
}

function openFilterPopover() {
  if (ws.bundleFiles.value.length === 0) return
  if (filterPopoverVisible.value) return
  syncDraftFileFilter()
  filterPopoverVisible.value = true
}

function normalizeTextList(values: string[] | undefined): string[] {
  const seen = new Set<string>()
  const result: string[] = []
  for (const value of values || []) {
    const trimmed = value.trim()
    if (!trimmed) continue
    const key = trimmed.toLocaleLowerCase()
    if (seen.has(key)) continue
    seen.add(key)
    result.push(trimmed)
  }
  return result
}

function togglePopularFileType(name: string) {
  if (draftFileFilter.typeNames.includes(name)) {
    draftFileFilter.typeNames = draftFileFilter.typeNames.filter(item => item !== name)
  } else {
    draftFileFilter.typeNames = [...draftFileFilter.typeNames, name]
  }
}

function applyFileFilter() {
  const nextFilter: FileFilterState = {
    ...draftFileFilter,
    typeNames: normalizeTextList(draftFileFilter.typeNames),
    includeQueries: normalizeTextList(draftFileFilter.includeQueries),
    excludeQueries: normalizeTextList(draftFileFilter.excludeQueries),
  }
  appliedFileFilter.value = nextFilter
  ws.setActiveFilters(nextFilter.typeNames, { allowScanFallback: true })
  filterPopoverVisible.value = false
  persistFileFilter().catch((error) => console.error('Failed to save File filter:', error))
}

function resetFileFilter() {
  appliedFileFilter.value = createDefaultFileFilter()
  Object.assign(draftFileFilter, createDefaultFileFilter())
  ws.setActiveFilters([], { allowScanFallback: false })
  filterPopoverVisible.value = false
  persistFileFilter().catch((error) => console.error('Failed to save File filter:', error))
}

function parseFileSizeBytes(value: string): number | null {
  const trimmed = value.trim()
  if (!trimmed) return null
  const parsed = Number(trimmed)
  if (!Number.isFinite(parsed) || parsed < 0) return null
  const multiplier = appliedFileFilter.value.sizeUnit === 'MB'
    ? 1024 * 1024
    : appliedFileFilter.value.sizeUnit === 'KB'
      ? 1024
      : 1
  return Math.floor(parsed * multiplier)
}

function matchesTextMode(source: string, name: string, query: string, mode: FileMatchMode): boolean {
  const needle = query.toLocaleLowerCase()
  const nameLower = name.toLocaleLowerCase()
  if (mode === 'starts_with') return nameLower.startsWith(needle)
  if (mode === 'ends_with') return nameLower.endsWith(needle)
  if (mode === 'equals') return nameLower === needle
  return source.toLocaleLowerCase().includes(needle)
}

function fileSearchText(file: BundleFileEntry): string {
  return `${file.name}\n${file.path}\n${getRelativeFilePath(file)}`
}

const typeFilteredFiles = computed(() => {
  if (appliedFileFilter.value.typeNames.length === 0) return [...ws.bundleFiles.value]
  const fileMap = new Map(ws.bundleFiles.value.map(file => [file.path, file]))
  return ws.highlightedFilePaths.value
    .map(path => fileMap.get(path))
    .filter((file): file is BundleFileEntry => file !== undefined)
})

const filteredFiles = computed(() => {
  const filter = appliedFileFilter.value
  const includes = normalizeTextList(filter.includeQueries)
  const excludes = normalizeTextList(filter.excludeQueries)
  const minSize = parseFileSizeBytes(filter.minSizeText)
  const maxSize = parseFileSizeBytes(filter.maxSizeText)

  let files = typeFilteredFiles.value.filter(file => {
    const searchText = fileSearchText(file)
    if (includes.some(query => !matchesTextMode(searchText, file.name, query, filter.matchMode))) return false
    if (excludes.some(query => matchesTextMode(searchText, file.name, query, 'contains'))) return false
    if (minSize !== null && file.size < minSize) return false
    if (maxSize !== null && file.size > maxSize) return false
    return true
  })

  const direction = filter.sortDirection === 'asc' ? 1 : -1
  files = [...files].sort((left, right) => {
    if (filter.sortBy === 'size') {
      const delta = (left.size - right.size) * direction
      if (delta !== 0) return delta
    } else if (filter.sortBy === 'modified') {
      const delta = (left.modified_ms - right.modified_ms) * direction
      if (delta !== 0) return delta
    } else if (filter.sortBy === 'name') {
      const delta = left.name.localeCompare(right.name) * direction
      if (delta !== 0) return delta
    } else {
      const delta = getRelativeFilePath(left).localeCompare(getRelativeFilePath(right)) * direction
      if (delta !== 0) return delta
    }
    return getRelativeFilePath(left).localeCompare(getRelativeFilePath(right))
  })

  return files
})

function makeDirectoryNode(name: string, relativePath: string, depth: number): FileTreeDirectoryNode {
  return {
    kind: 'dir',
    id: `dir:${relativePath}`,
    name,
    relativePath,
    depth,
    children: [],
    fileCount: 0,
    totalSize: 0,
  }
}

function sortFileTreeNodes(nodes: FileTreeNode[]): FileTreeNode[] {
  return nodes.sort((left, right) => {
    if (left.kind !== right.kind) return left.kind === 'dir' ? -1 : 1
    if (left.kind === 'dir' && right.kind === 'dir') {
      return left.name.localeCompare(right.name)
    }
    return left.name.localeCompare(right.name)
  })
}

function sortFileTreeRecursively(nodes: FileTreeNode[]): FileTreeNode[] {
  for (const node of nodes) {
    if (node.kind === 'dir') sortFileTreeRecursively(node.children)
  }
  return sortFileTreeNodes(nodes)
}

const fileTreeNodes = computed<FileTreeNode[]>(() => {
  const roots: FileTreeNode[] = []
  const directories = new Map<string, FileTreeDirectoryNode>()

  for (const file of filteredFiles.value) {
    const parts = getRelativeFilePath(file).split('/').filter(Boolean)
    const fileName = parts.pop() || file.name
    let children = roots
    let relativePath = ''
    const ancestorNodes: FileTreeDirectoryNode[] = []

    for (const part of parts) {
      relativePath = relativePath ? `${relativePath}/${part}` : part
      const id = `dir:${relativePath}`
      let dir = directories.get(id)
      if (!dir) {
        dir = makeDirectoryNode(part, relativePath, ancestorNodes.length)
        directories.set(id, dir)
        children.push(dir)
      }
      dir.fileCount += 1
      dir.totalSize += file.size
      ancestorNodes.push(dir)
      children = dir.children
    }

    children.push({
      kind: 'file',
      id: `file:${file.path}`,
      name: fileName,
      depth: parts.length,
      file,
    })
  }

  return sortFileTreeRecursively(roots)
})

function flattenVisibleFileTree(nodes: FileTreeNode[]): FileTreeNode[] {
  const result: FileTreeNode[] = []
  for (const node of nodes) {
    result.push(node)
    if (node.kind === 'dir' && isDirectoryExpanded(node)) {
      result.push(...flattenVisibleFileTree(node.children))
    }
  }
  return result
}

const visibleFileTreeNodes = computed(() => flattenVisibleFileTree(fileTreeNodes.value))

const visibleFileNodes = computed(() => {
  return visibleFileTreeNodes.value.filter((node): node is FileTreeFileNode => node.kind === 'file')
})

function isDirectoryExpanded(node: FileTreeDirectoryNode): boolean {
  return expandedDirectories.value.has(node.id)
}

function toggleDirectory(node: FileTreeDirectoryNode) {
  const next = new Set(expandedDirectories.value)
  if (next.has(node.id)) {
    next.delete(node.id)
  } else {
    next.add(node.id)
  }
  expandedDirectories.value = next
  persistExpandedDirectories().catch((error) => console.error('Failed to save expanded File directories:', error))
}

function selectTreeFile(file: BundleFileEntry) {
  ws.setKeyboardNavigationScope('files')
  ws.selectBundle(file)
}

async function selectVisibleFileByIndex(index: number) {
  const nodes = visibleFileNodes.value
  if (index < 0 || index >= nodes.length) return
  await selectTreeFile(nodes[index].file)
  await nextTick()
  const container = ws.fileListRef.value
  const items = container?.querySelectorAll('.file-item')
  const target = items?.[index] as HTMLElement | undefined
  target?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
}

function handleFileTreeKeydown(event: KeyboardEvent): boolean {
  ws.setKeyboardNavigationScope('files')
  const nodes = visibleFileNodes.value
  if (nodes.length === 0) return false

  if (event.key === 'ArrowDown') {
    event.preventDefault()
    const currentIndex = nodes.findIndex(node => node.file.path === ws.selectedBundlePath.value)
    selectVisibleFileByIndex(currentIndex >= 0 && currentIndex < nodes.length - 1 ? currentIndex + 1 : 0)
    return true
  } else if (event.key === 'ArrowUp') {
    event.preventDefault()
    const currentIndex = nodes.findIndex(node => node.file.path === ws.selectedBundlePath.value)
    selectVisibleFileByIndex(currentIndex > 0 ? currentIndex - 1 : nodes.length - 1)
    return true
  }
  return false
}

function getDirectoryInfo(node: FileTreeDirectoryNode): string {
  return `${node.fileCount} / ${FormatUtils.formatSize(node.totalSize)}`
}

/**
 * Open this file in the file manager and reveal it.
 *
 * Calls the Tauri revealItemInDir API,
 * selects the file in Windows Explorer, reveals and highlights it in macOS Finder.
 */
async function openFileInExplorer() {
  const file = contextMenuFile.value
  if (!file) return

  hideContextMenu()
  try {
    await revealItemInDir(file.path)
  } catch (e) {
    console.error('Failed to open file manager:', e)
  }
}
</script>

<template>
  <!-- ====== Header: file list title + type filter ====== -->
  <div class="pane-header">
    <span class="pane-title">
      <span>{{ t('home.fileList') }}</span>
      <span class="panel-count" v-if="ws.bundleFiles.value.length > 0">
        {{ filteredFiles.length }}/{{ ws.bundleFiles.value.length }}
      </span>
    </span>
    <span class="pane-header-right">
      <button
        class="filter-icon-btn filter-text-btn"
        :class="{ active: filterPopoverVisible || fileFilterActiveCount > 0 }"
        :disabled="ws.bundleFiles.value.length === 0"
        :title="t('filterCard.fileTitle')"
        @click.stop="openFilterPopover"
      >
        <span class="filter-btn-label">{{ t('filterCard.advancedTitle') }}</span>
        <span v-if="fileFilterActiveCount > 0" class="filter-count">
          {{ fileFilterActiveCount }}
        </span>
      </button>
    </span>
  </div>

  <CenteredFilterModal
    :visible="filterPopoverVisible"
    width="min(820px, calc(100vw - 32px))"
    @update:visible="filterPopoverVisible = $event"
  >
    <div class="advanced-filter-card" @click.stop>
      <header class="advanced-filter-header">
        <div>
          <h3>{{ t('filterCard.fileTitle') }}</h3>
          <p>{{ t('filterCard.fileSubtitle') }}</p>
        </div>
        <div class="advanced-filter-actions">
          <el-button size="small" :icon="Refresh" @click="resetFileFilter">{{ t('filterCard.reset') }}</el-button>
          <el-button size="small" type="primary" :icon="Search" @click="applyFileFilter">{{ t('filterCard.apply') }}</el-button>
        </div>
      </header>

      <section class="filter-section">
        <div class="filter-section-title">{{ t('filterCard.containedAssetType') }}</div>
        <el-select
          v-model="draftFileFilter.typeNames"
          multiple
          filterable
          :teleported="false"
          clearable
          :placeholder="t('filterCard.anyType')"
          class="wide-control"
        >
          <el-option
            v-for="stat in fileTypeStats"
            :key="stat.name"
            :label="`${stat.name}${stat.count > 0 ? ` (${stat.count})` : ''}`"
            :value="stat.name"
          />
        </el-select>
        <div class="type-chip-row">
          <button
            v-for="stat in popularFileTypes"
            :key="stat.name"
            class="type-chip"
            :class="{ active: draftFileFilter.typeNames.includes(stat.name) }"
            @click="togglePopularFileType(stat.name)"
          >
            <el-icon :size="14"><component :is="ClassIconUtils.getClassIcon(stat.name)" /></el-icon>
            <span>{{ stat.name }}</span>
          </button>
        </div>
      </section>

      <section class="filter-grid">
        <div class="filter-field span-2">
          <span>{{ t('filterCard.fileNamePathContainsAll') }}</span>
          <div class="inline-controls">
            <el-select v-model="draftFileFilter.matchMode" :teleported="false" class="mode-select">
              <el-option :label="t('filterCard.match.contains')" value="contains" />
              <el-option :label="t('filterCard.match.startsWith')" value="starts_with" />
              <el-option :label="t('filterCard.match.endsWith')" value="ends_with" />
              <el-option :label="t('filterCard.match.equals')" value="equals" />
            </el-select>
            <el-input-tag
              v-model="draftFileFilter.includeQueries"
              clearable
              :delimiter="/[,;，；]/"
              class="wide-control"
              tag-type="primary"
              :placeholder="t('filterCard.requiredTermsPlaceholder')"
              @blur="draftFileFilter.includeQueries = normalizeTextList(draftFileFilter.includeQueries)"
            />
          </div>
        </div>

        <div class="filter-field span-2">
          <span>{{ t('filterCard.fileNamePathExcludeAny') }}</span>
          <el-input-tag
            v-model="draftFileFilter.excludeQueries"
            clearable
            :delimiter="/[,;，；]/"
            class="wide-control"
            tag-type="warning"
            :placeholder="t('filterCard.forbiddenTermsPlaceholder')"
            @blur="draftFileFilter.excludeQueries = normalizeTextList(draftFileFilter.excludeQueries)"
          />
        </div>

        <div class="filter-field">
          <span>{{ t('filterCard.minimumSize') }}</span>
          <el-input v-model="draftFileFilter.minSizeText" clearable placeholder="0" />
        </div>

        <div class="filter-field">
          <span>{{ t('filterCard.maximumSize') }}</span>
          <div class="inline-controls">
            <el-input v-model="draftFileFilter.maxSizeText" clearable :placeholder="t('filterCard.anyValue')" />
            <el-select v-model="draftFileFilter.sizeUnit" :teleported="false" class="unit-select">
              <el-option label="B" value="B" />
              <el-option label="KB" value="KB" />
              <el-option label="MB" value="MB" />
            </el-select>
          </div>
        </div>

        <div class="filter-field">
          <span>{{ t('filterCard.sortBy') }}</span>
          <el-select v-model="draftFileFilter.sortBy" :teleported="false">
            <el-option :label="t('filterCard.sort.path')" value="path" />
            <el-option :label="t('filterCard.sort.name')" value="name" />
            <el-option :label="t('filterCard.sort.size')" value="size" />
            <el-option :label="t('filterCard.sort.modified')" value="modified" />
          </el-select>
        </div>

        <div class="filter-field">
          <span>{{ t('filterCard.direction') }}</span>
          <el-segmented
            v-model="draftFileFilter.sortDirection"
            :options="[
              { label: t('filterCard.directionAsc'), value: 'asc' },
              { label: t('filterCard.directionDesc'), value: 'desc' },
            ]"
          />
        </div>
      </section>
    </div>
  </CenteredFilterModal>

  <!-- ====== File list scroll container ====== -->
  <div
    :ref="setFileListElement"
    class="pane-scroll pane-files"
    tabindex="0"
    @focusin="ws.setKeyboardNavigationScope('files')"
    @pointerdown="ws.setKeyboardNavigationScope('files')"
    @keydown="handleFileTreeKeydown"
  >
    <!-- Loading indicator when directory or filter scan is in progress -->
    <div v-if="ws.isLoading.value || ws.isFilterLoading.value" class="panel-loading">
      <el-icon class="is-loading" :size="24"><Refresh /></el-icon>
      <p>{{ ws.isLoading.value ? (ws.loadingText.value || t('workspaceLog.scanningFiles')) : t('home.scanningTypes') }}</p>
    </div>

    <!-- Empty state prompt -->
    <div v-else-if="ws.bundleFiles.value.length === 0" class="panel-empty">
      {{ t('home.selectBundleDirectory') }}
    </div>

    <!-- File tree items -->
    <template
      v-for="node in visibleFileTreeNodes"
      :key="node.id"
    >
      <div
        v-if="node.kind === 'dir'"
        class="file-tree-dir"
        :style="{ paddingLeft: `${8 + node.depth * 16}px` }"
        @click="toggleDirectory(node)"
      >
        <el-icon class="file-tree-arrow">
          <ArrowDown v-if="isDirectoryExpanded(node)" />
          <ArrowRight v-else />
        </el-icon>
        <el-icon><Folder /></el-icon>
        <span class="file-tree-dir-name">{{ node.name }}</span>
        <span class="file-tree-dir-info">{{ getDirectoryInfo(node) }}</span>
      </div>

      <div
        v-else
        :class="[
          'file-item',
          'file-tree-file',
          { 'file-item-active': node.file.path === ws.selectedBundlePath.value },
          { 'file-item-matches': ws.fileMatchesFilter(node.file.path) },
        ]"
        :style="{ paddingLeft: `${28 + node.depth * 16}px` }"
        @click="selectTreeFile(node.file)"
        @contextmenu="showContextMenu($event, node.file)"
      >
        <div class="file-item-name">
          <el-icon><Folder /></el-icon>
          {{ node.name }}
        </div>
        <div class="file-item-info">
          {{ FormatUtils.formatSize(node.file.size) }}
          <el-tag
            v-if="ws.fileMatchesFilter(node.file.path)"
            size="small"
            type="success"
            effect="dark"
            class="match-badge"
          >
            {{ getMatchLabel(node.file.path) }}
          </el-tag>
        </div>
      </div>
    </template>
  </div>
  <!-- ====== Custom context menu ====== -->
  <teleport to="body">
    <!-- Overlay: click blank area to close menu -->
    <div
      v-if="contextMenuVisible"
      class="file-context-menu-overlay"
      @click="hideContextMenu"
      @contextmenu.prevent="hideContextMenu"
    ></div>

    <!-- Menu panel -->
    <div
      v-if="contextMenuVisible"
      ref="contextMenuRef"
      class="file-context-menu"
      :style="{ left: contextMenuPos.x + 'px', top: contextMenuPos.y + 'px' }"
      @click.stop
      @contextmenu.prevent.stop
    >
      <div class="file-context-menu-item" @click="openFileInExplorer">
        <el-icon class="file-context-menu-icon"><Folder /></el-icon>
        {{ t('home.revealFile') }}
      </div>
    </div>
  </teleport>
</template>

<style>
/* File list panel styles inherited from Home.vue global styles */

/*
 * Context menu styles
 * Uses position: fixed to position at mouse click location
 * Note: this style is not scoped because <teleport to="body"> renders the menu outside the component.
 */
.file-context-menu-overlay {
  position: fixed;
  inset: 0;
  z-index: 9998;
  background: transparent;
}

.file-context-menu {
  position: fixed;
  z-index: 9999;
  min-width: 220px;
  padding: 4px 0;
  background: var(--el-bg-color);
  border: 1px solid var(--el-border-color-light);
  border-radius: 6px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.15);
  font-size: 13px;
  overflow: hidden;
}

.file-context-menu-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 16px;
  cursor: pointer;
  color: var(--el-text-color-primary);
  transition: background 0.12s;
  user-select: none;
}

.file-context-menu-item:hover {
  background: var(--el-fill-color-light);
  color: var(--el-color-primary);
}

.file-context-menu-icon {
  font-size: 14px;
  width: 18px;
  flex-shrink: 0;
}

.file-tree-dir {
  display: flex;
  align-items: center;
  gap: 4px;
  min-height: 30px;
  padding-top: 6px;
  padding-right: 12px;
  padding-bottom: 6px;
  cursor: pointer;
  border-bottom: 1px solid var(--el-border-color-extra-light);
  color: var(--el-text-color-primary);
  font-size: 13px;
  user-select: none;
  transition: background 0.15s ease;
}

.file-tree-dir:hover {
  background: var(--el-fill-color-light);
}

.file-tree-arrow {
  width: 14px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  flex-shrink: 0;
}

.file-tree-dir-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.file-tree-dir-info {
  margin-left: auto;
  flex-shrink: 0;
  color: var(--el-text-color-secondary);
  font-size: 11px;
}

.file-tree-file {
  padding-right: 12px;
}

.advanced-filter-card {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 4px;
}

.advanced-filter-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  border-bottom: 1px solid var(--el-border-color-lighter);
  padding-bottom: 12px;
}

.advanced-filter-header h3 {
  margin: 0;
  font-size: 15px;
  line-height: 1.2;
}

.advanced-filter-header p {
  margin: 4px 0 0;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.advanced-filter-actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}

.filter-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.filter-section-title,
.filter-field > span {
  font-size: 12px;
  font-weight: 650;
  color: var(--el-text-color-primary);
}

.wide-control {
  width: 100%;
}

:deep(.el-input-tag.wide-control) {
  min-height: 32px;
}

:deep(.el-input-tag.wide-control .el-input-tag__inner) {
  align-items: flex-start;
  flex-wrap: wrap;
  row-gap: 5px;
}

.type-chip-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.type-chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 26px;
  padding: 0 8px;
  border: 1px solid var(--el-border-color);
  border-radius: 4px;
  background: var(--el-bg-color);
  color: var(--el-text-color-regular);
  font-size: 12px;
  cursor: pointer;
}

.type-chip.active,
.type-chip:hover {
  border-color: var(--el-color-primary);
  color: var(--el-color-primary);
  background: var(--el-color-primary-light-9);
}

.filter-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.filter-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.span-2 {
  grid-column: span 2;
}

.inline-controls {
  display: flex;
  gap: 8px;
  min-width: 0;
}

.mode-select {
  width: 136px;
  flex-shrink: 0;
}

.unit-select {
  width: 82px;
  flex-shrink: 0;
}

@media (max-width: 900px) {
  .filter-grid {
    grid-template-columns: 1fr;
  }

  .span-2 {
    grid-column: span 1;
  }
}
</style>
