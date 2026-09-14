import { ref } from 'vue'
import { load } from '@tauri-apps/plugin-store'

const logHeight = ref(200)
const draggingLogHeight = ref(false)

let dragStartY = 0
let dragStartHeight = 0
let saveTimer: ReturnType<typeof setTimeout> | null = null

export function useGlobalLogLayout() {
  function startDragLogHeight(event: MouseEvent) {
    draggingLogHeight.value = true
    dragStartY = event.clientY
    dragStartHeight = logHeight.value
    document.addEventListener('mousemove', onDragMove)
    document.addEventListener('mouseup', onDragEnd)
  }

  function onDragMove(event: MouseEvent) {
    if (!draggingLogHeight.value) return
    const delta = dragStartY - event.clientY
    logHeight.value = Math.max(80, Math.min(520, dragStartHeight + delta))
  }

  function onDragEnd() {
    draggingLogHeight.value = false
    document.removeEventListener('mousemove', onDragMove)
    document.removeEventListener('mouseup', onDragEnd)
    debounceSaveLogLayout()
  }

  function debounceSaveLogLayout() {
    if (saveTimer) clearTimeout(saveTimer)
    saveTimer = setTimeout(async () => {
      try {
        const s = await load('settings.json', { defaults: {}, autoSave: true })
        await s.set('global-log-layout', { logHeight: logHeight.value })
        await s.save()
      } catch (e) {
        console.error('Failed to save log layout:', e)
      }
    }, 500)
  }

  async function loadLogLayout() {
    try {
      const s = await load('settings.json', { defaults: {}, autoSave: true })
      const saved = await s.get('global-log-layout')
      if (saved && typeof saved === 'object') {
        const layout = saved as Record<string, number>
        if (layout.logHeight) logHeight.value = layout.logHeight
      }
    } catch (e) {
      console.error('Failed to load log layout:', e)
    }
  }

  return {
    logHeight,
    draggingLogHeight,
    startDragLogHeight,
    loadLogLayout,
  }
}
