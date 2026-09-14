/**
 * usePanelLayout ->Draggable splitter + panel/window size persistence composable
 *
 * Provides three draggable splitters:
 *   1. Left/right panels (vertical splitter)
 *   2. Left side file list/asset list (vertical splitter)
 *   3. Right side preview/properties (horizontal splitter)
 *
 * All panel sizes are debounced and saved to Tauri store after dragging ends,
 * automatically restored on next startup. Window size is also auto-saved on resize.
 *
 * Usage:
 *   const layout = usePanelLayout()
 *   // Bind layout.leftPanelWidth, layout.startDragLeftRight, etc. in template
 */

import { ref } from 'vue'
import { load } from '@tauri-apps/plugin-store'

// ============================================================
// Module-level state (singleton)
// ============================================================

/* ---------- Panel Sizes ---------- */

/** Left panel width (pixels), default 680 (file+asset side-by-side needs more width) */
const leftPanelWidth = ref(680)

/** File list width inside the files tab (pixels), default 260 */
const fileListWidth = ref(260)

/** Preview area height in right panel (pixels), default 350 */
const previewHeight = ref(350)

/* ---------- Drag State ---------- */

/** Currently active drag splitter type, null means no active drag */
const activeDrag = ref<'left-right' | 'files-assets' | 'right-vertical' | null>(null)

/** Mouse position at drag start (clientX or clientY), used to compute delta */
let dragStartPos = 0

/** Panel size at drag start, used to accumulate delta */
let dragStartSize = 0

/** Main layout area DOM reference, used to compute boundaries for left-right splitter */
const mainLayoutRef = ref<HTMLDivElement | null>(null)

/** Main layout width cache, used for boundary clamping during left-right drag */
let mainLayoutWidth = 0

// ============================================================
// Exported composable function
// ============================================================

export function usePanelLayout() {
  // ==========================================================
  // Draggable splitter
  // ==========================================================

  /**
   * Start dragging the left-right panel splitter.
   *
   * Triggered when mouse is pressed on the vertical splitter, records start position
   * and panel size, then listens to global mousemove/mouseup for the drag interaction.
   */
  function startDragLeftRight(event: MouseEvent) {
    activeDrag.value = 'left-right'
    dragStartPos = event.clientX
    dragStartSize = leftPanelWidth.value
    // Cache current main layout width for computing minimum right panel width
    mainLayoutWidth = mainLayoutRef.value?.clientWidth || 1200
    document.addEventListener('mousemove', onDragMove)
    document.addEventListener('mouseup', onDragEnd)
  }

  /**
   * Start dragging the vertical splitter inside the right panel (preview area / properties panel).
   */
  function startDragRightVertical(event: MouseEvent) {
    activeDrag.value = 'right-vertical'
    dragStartPos = event.clientY
    dragStartSize = previewHeight.value
    document.addEventListener('mousemove', onDragMove)
    document.addEventListener('mouseup', onDragEnd)
  }

  /**
   * Start dragging the splitter between the file list and asset list.
   */
  function startDragFilesAssets(event: MouseEvent) {
    activeDrag.value = 'files-assets'
    dragStartPos = event.clientX
    dragStartSize = fileListWidth.value
    document.addEventListener('mousemove', onDragMove)
    document.addEventListener('mouseup', onDragEnd)
  }

  /**
   * Handle mouse movement during drag.
   *
   * Computes delta based on the active drag type and updates the corresponding panel size.
   * Each direction has reasonable min/max constraints to prevent panels from being dragged out of view.
   */
  function onDragMove(event: MouseEvent) {
    switch (activeDrag.value) {
      case 'left-right': {
        // Left panel width range: 400px ~ main layout width - minimum right panel width (400px)
        const delta = event.clientX - dragStartPos
        const minWidth = 400
        const maxWidth = mainLayoutWidth - 400
        leftPanelWidth.value = Math.max(minWidth, Math.min(maxWidth, dragStartSize + delta))
        break
      }
      case 'files-assets': {
        const delta = event.clientX - dragStartPos
        const minWidth = 180
        const maxWidth = Math.max(minWidth, leftPanelWidth.value - 260)
        fileListWidth.value = Math.max(minWidth, Math.min(maxWidth, dragStartSize + delta))
        break
      }
      case 'right-vertical': {
        // Right upper section (preview area) height range: 150px ~ right panel height - minimum properties height (100px)
        const delta = event.clientY - dragStartPos
        previewHeight.value = Math.max(150, Math.min(1000, dragStartSize + delta))
        break
      }
    }
  }

  /**
   * Drag end: remove global event listeners, debounce-save panel layout to store.
   */
  function onDragEnd() {
    activeDrag.value = null
    document.removeEventListener('mousemove', onDragMove)
    document.removeEventListener('mouseup', onDragEnd)
    // Debounce save to avoid frequent store writes
    debounceSavePanelLayout()
  }

  // ==========================================================
  // Panel layout persistence
  // ==========================================================

  /** Debounce save timer handle */
  let saveLayoutTimer: ReturnType<typeof setTimeout> | null = null

  /**
   * Debounce-save panel layout to store.
   *
   * Writes to disk 500ms after drag ends to avoid frequent IO.
   * If the user starts dragging again within 500ms, the previous timer is cleared.
   */
  async function debounceSavePanelLayout() {
    if (saveLayoutTimer) clearTimeout(saveLayoutTimer)
    saveLayoutTimer = setTimeout(async () => {
      try {
        const s = await load('settings.json', { defaults: {}, autoSave: true })
        await s.set('panel-layout', {
          leftPanelWidth: leftPanelWidth.value,
          fileListWidth: fileListWidth.value,
          previewHeight: previewHeight.value,
        })
        // Explicit save() to ensure immediate write to disk
        await s.save()
      } catch (e) {
        console.error('Failed to save panel layout:', e)
      }
    }, 500)
  }

  /**
   * Load previously saved panel layout from store.
   *
   * Called on component mount to restore the user's last adjusted panel sizes.
   */
  async function loadPanelLayout() {
    try {
      const s = await load('settings.json', { defaults: {}, autoSave: true })
      const saved = await s.get('panel-layout')
      if (saved && typeof saved === 'object') {
        const layout = saved as Record<string, number>
        if (layout.leftPanelWidth) leftPanelWidth.value = layout.leftPanelWidth
        if (layout.fileListWidth) fileListWidth.value = layout.fileListWidth
        if (layout.previewHeight) previewHeight.value = layout.previewHeight
      }
    } catch (e) {
      console.error('Failed to load panel layout:', e)
    }
  }

  // ==========================================================
  // Return values
  // ==========================================================

  return {
    /* Panel sizes */
    leftPanelWidth,
    fileListWidth,
    previewHeight,

    /* Drag state */
    activeDrag,
    mainLayoutRef,

    /* Drag methods */
    startDragLeftRight,
    startDragFilesAssets,
    startDragRightVertical,

    /* Persistence methods */
    loadPanelLayout,
  }
}
