<script setup lang="ts">
/**
 * LogPanel - Bottom Log Panel
 *
 * Shows operation logs, supports export, clear, and stopping all background tasks.
 * Logs auto-scroll to bottom, rendering the last 500 entries while counting full history.
 */

import { computed, ref, nextTick, onMounted, onBeforeUnmount } from 'vue'
import { Delete, Download } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import { useLogSystem } from '../composables/useLogSystem'
import type { LogEntry, TaskProgress } from '../composables/useLogSystem'

const { t } = useI18n()
const {
  logs,
  totalLogCount,
  logContainerRef,
  runningTaskCount,
  taskProgressMap,
  runningTaskMap,
  stopAllTasks,
  refreshRunningTaskCount,
  clearLogs,
  exportLogs,
  buildAllLogText,
  addLog,
} = useLogSystem()

/** Use computed to ensure reactive new key changes are correctly tracked by Vue */
const progressList = computed<TaskProgress[]>(() => Object.values(taskProgressMap))
const runningTaskList = computed(() => Object.values(runningTaskMap))
const shouldShowStopButton = computed(() => {
  return runningTaskCount.value > 0 || runningTaskList.value.length > 0 || progressList.value.length > 0
})
const primaryRunningTask = computed(() => {
  if (runningTaskList.value.length === 0) return null
  return [...runningTaskList.value].sort((leftTask, rightTask) => rightTask.started_at_ms - leftTask.started_at_ms)[0]
})
const primaryRunningSummary = computed(() => {
  const task = primaryRunningTask.value
  if (!task) return ''
  return `${task.task_label} - ${taskElapsed(task.started_at_ms)}`
})
const exportingLogs = ref(false)
const nowMs = ref(Date.now())
const allLogSelectBuffer = ref<HTMLTextAreaElement | null>(null)
const logContextMenuRef = ref<HTMLElement | null>(null)
const logSuccessToastRef = ref<HTMLElement | null>(null)
const logContextMenu = ref({
  visible: false,
  x: 0,
  y: 0,
  lineText: '',
  selectedText: '',
})
const logSuccessToast = ref({
  visible: false,
  x: 0,
  y: 0,
  text: '',
})
const allLogsSelected = ref(false)
let logSuccessToastTimer: ReturnType<typeof setTimeout> | null = null

function formatVisibleLogLine(log: LogEntry) {
  return `[${log.time}] ${log.message}`
}

function formatDuration(totalSeconds: number) {
  const seconds = Math.max(0, Math.floor(totalSeconds))
  const h = Math.floor(seconds / 3600)
  const m = Math.floor((seconds % 3600) / 60)
  const s = seconds % 60
  return [h, m, s].map(v => String(v).padStart(2, '0')).join(':')
}

function taskElapsed(startedAtMs: number) {
  return formatDuration((nowMs.value - startedAtMs) / 1000)
}

async function copyText(text: string) {
  if (!text) return

  try {
    await navigator.clipboard.writeText(text)
  } catch {
    const textarea = document.createElement('textarea')
    textarea.value = text
    textarea.style.position = 'fixed'
    textarea.style.left = '-9999px'
    textarea.style.top = '0'
    document.body.appendChild(textarea)
    textarea.focus()
    textarea.select()
    document.execCommand('copy')
    document.body.removeChild(textarea)
  }
}

function getSelectedText() {
  return window.getSelection()?.toString() || ''
}

function clampLogContextMenuPosition() {
  const menu = logContextMenuRef.value
  if (!menu || !logContextMenu.value.visible) return

  const gap = 8
  const width = menu.offsetWidth
  const height = menu.offsetHeight
  const sourceX = logContextMenu.value.x
  const sourceY = logContextMenu.value.y
  const maxX = window.innerWidth - width - gap
  const maxY = window.innerHeight - height - gap

  logContextMenu.value.x = Math.max(gap, Math.min(sourceX, maxX))
  logContextMenu.value.y = sourceY + height + gap > window.innerHeight
    ? Math.max(gap, sourceY - height - gap)
    : Math.max(gap, Math.min(sourceY, maxY))
}

function showLogContextMenu(event: MouseEvent, log?: LogEntry) {
  logContextMenu.value = {
    visible: true,
    x: event.clientX,
    y: event.clientY,
    lineText: log ? formatVisibleLogLine(log) : '',
    selectedText: getSelectedText(),
  }
  nextTick(clampLogContextMenuPosition)
}

function clampLogSuccessToastPosition(anchorX: number, anchorY: number) {
  const toast = logSuccessToastRef.value
  if (!toast || !logSuccessToast.value.visible) return

  const gap = 8
  const offset = 12
  const width = toast.offsetWidth
  const height = toast.offsetHeight
  const rightX = anchorX + offset
  const bottomY = anchorY + offset

  logSuccessToast.value.x = rightX + width + gap > window.innerWidth
    ? Math.max(gap, anchorX - width - offset)
    : Math.max(gap, rightX)
  logSuccessToast.value.y = bottomY + height + gap > window.innerHeight
    ? Math.max(gap, anchorY - height - offset)
    : Math.max(gap, bottomY)
}

function showLogSuccessToast(text: string, event?: MouseEvent) {
  if (logSuccessToastTimer) {
    clearTimeout(logSuccessToastTimer)
    logSuccessToastTimer = null
  }

  const anchorX = event?.clientX ?? logContextMenu.value.x
  const anchorY = event?.clientY ?? logContextMenu.value.y
  logSuccessToast.value = {
    visible: true,
    x: anchorX + 12,
    y: anchorY + 12,
    text,
  }
  nextTick(() => clampLogSuccessToastPosition(anchorX, anchorY))
  logSuccessToastTimer = setTimeout(() => {
    logSuccessToast.value.visible = false
    logSuccessToastTimer = null
  }, 1800)
}

function hideLogContextMenu() {
  logContextMenu.value.visible = false
}

async function copyContextLine(event: MouseEvent) {
  await copyText(logContextMenu.value.lineText)
  showLogSuccessToast(t('logPanel.copyLineSuccess'), event)
  hideLogContextMenu()
}

async function copyContextSelection(event: MouseEvent) {
  await copyText(logContextMenu.value.selectedText)
  showLogSuccessToast(t('logPanel.copySelectionSuccess'), event)
  hideLogContextMenu()
}

function selectAllLogs(event: MouseEvent) {
  allLogsSelected.value = true
  showLogSuccessToast(t('logPanel.selectAllLogsSuccess'), event)
  hideLogContextMenu()
  nextTick(() => {
    const buffer = allLogSelectBuffer.value
    if (!buffer) return
    buffer.value = buildAllLogText()
    buffer.focus()
    buffer.select()
  })
}

async function copyAllLogs(event: MouseEvent) {
  await copyText(buildAllLogText())
  showLogSuccessToast(t('logPanel.copyAllLogsSuccess'), event)
  hideLogContextMenu()
}

function clearAllLogSelection() {
  allLogsSelected.value = false
}

// ============================================================
// Task Count Polling
// ============================================================

let taskCountTimer: ReturnType<typeof setInterval> | null = null
let clockTimer: ReturnType<typeof setInterval> | null = null

function startTaskCountPolling() {
  if (taskCountTimer) return
  refreshRunningTaskCount()
  taskCountTimer = setInterval(refreshRunningTaskCount, 2000)
}

function stopTaskCountPolling() {
  if (taskCountTimer) { clearInterval(taskCountTimer); taskCountTimer = null }
}

function startRunningClock() {
  if (clockTimer) return
  nowMs.value = Date.now()
  clockTimer = setInterval(() => {
    nowMs.value = Date.now()
  }, 1000)
}

function stopRunningClock() {
  if (clockTimer) { clearInterval(clockTimer); clockTimer = null }
}

async function handleStopAll() { await stopAllTasks() }

async function handleExportLogs(event?: MouseEvent) {
  if (totalLogCount.value === 0 || exportingLogs.value) return

  exportingLogs.value = true
  try {
    const filePath = await exportLogs()
    if (filePath) {
      addLog(t('logPanel.logsExported', { path: filePath }), 'success')
      showLogSuccessToast(t('logPanel.exportLogsSuccess'), event)
    }
  } catch (e) {
    addLog(t('logPanel.exportLogsFailed', { error: e }), 'error')
  } finally {
    exportingLogs.value = false
  }
}

onMounted(() => {
  startTaskCountPolling()
  startRunningClock()
  document.addEventListener('mousedown', hideLogContextMenu)
  document.addEventListener('mousedown', clearAllLogSelection)
  document.addEventListener('keydown', hideLogContextMenu)
  document.addEventListener('keydown', clearAllLogSelection)
})

onBeforeUnmount(() => {
  if (logSuccessToastTimer) clearTimeout(logSuccessToastTimer)
  stopTaskCountPolling()
  stopRunningClock()
  document.removeEventListener('mousedown', hideLogContextMenu)
  document.removeEventListener('mousedown', clearAllLogSelection)
  document.removeEventListener('keydown', hideLogContextMenu)
  document.removeEventListener('keydown', clearAllLogSelection)
})
</script>

<template>
  <div class="log-panel">
    <textarea
      ref="allLogSelectBuffer"
      class="log-select-buffer"
      readonly
      aria-hidden="true"
      tabindex="-1"
    ></textarea>

    <!-- Log Header -->
    <div class="log-header">
      <span class="log-title">
        {{ t('logPanel.title') }}
        <span class="log-count" v-if="totalLogCount > 0">{{ totalLogCount }}</span>
        <span
          class="log-running-badge"
          v-if="primaryRunningTask"
          :title="primaryRunningTask.task_id"
        >{{ primaryRunningSummary }}</span>
      </span>

      <div class="log-header-actions" @click.stop>
        <button
          v-if="shouldShowStopButton"
          class="log-stop-btn"
          :title="t('logPanel.stopTasks')"
          @click="handleStopAll"
        >
          <svg viewBox="0 0 16 16" width="14" height="14">
            <rect x="3" y="3" width="10" height="10" rx="1.5" fill="currentColor" />
          </svg>
        </button>
        <el-tooltip :content="t('logPanel.exportLogs')" placement="top">
          <button
            type="button"
            class="log-icon-btn"
            :title="t('logPanel.exportLogs')"
            :disabled="totalLogCount === 0 || exportingLogs"
            @click="handleExportLogs($event)"
          >
            <el-icon v-if="!exportingLogs"><Download /></el-icon>
            <span v-else class="log-icon-loading"></span>
          </button>
        </el-tooltip>
        <el-tooltip :content="t('logPanel.clearLogs')" placement="top">
          <button
            type="button"
            class="log-icon-btn"
            :title="t('logPanel.clearLogs')"
            :disabled="totalLogCount === 0"
            @click="clearLogs"
          >
            <el-icon><Delete /></el-icon>
          </button>
        </el-tooltip>
      </div>
    </div>

    <!-- Task Progress Bar Area -->
    <div class="log-progress-area" v-if="progressList.length > 0" @click.stop>
      <div
        v-for="p in progressList"
        :key="p.progress_key"
        class="log-progress-item"
      >
        <span class="log-progress-label" :title="`${p.task_label} - ${p.step}`">
          {{ p.task_label }} - {{ p.step }}
        </span>
        <span v-if="p.remaining_label" class="log-progress-eta">
          {{ t('logPanel.roughRemainingTime') }}: {{ p.remaining_label }}
        </span>
        <span v-else class="log-progress-eta log-progress-eta-empty"></span>
        <div class="log-progress-meter">
          <div class="log-progress-bar-track">
            <div
              class="log-progress-bar-fill"
              :style="{ width: (p.total > 0 ? (p.current / p.total) * 100 : 0) + '%' }"
            ></div>
          </div>
          <span class="log-progress-text">{{ p.current }}/{{ p.total }}</span>
        </div>
      </div>
    </div>

    <!-- Log Content Area -->
    <div
      ref="logContainerRef"
      :class="['log-body', { 'log-body-all-selected': allLogsSelected }]"
      @scroll="hideLogContextMenu"
      @contextmenu.prevent.stop="showLogContextMenu($event)"
    >
      <div v-if="logs.length === 0" class="log-empty">{{ t('logPanel.noLogs') }}</div>
      <div
        v-for="(log, i) in logs"
        :key="i"
        :class="['log-line', `log-${log.type}`]"
        @contextmenu.prevent.stop="showLogContextMenu($event, log)"
      >
        <span class="log-time">[{{ log.time }}]</span>
        <span class="log-msg">{{ log.message }}</span>
      </div>
    </div>

    <div
      v-if="logContextMenu.visible"
      ref="logContextMenuRef"
      class="log-context-menu"
      :style="{ left: `${logContextMenu.x}px`, top: `${logContextMenu.y}px` }"
      @mousedown.stop
      @contextmenu.prevent.stop
    >
      <button
        type="button"
        class="log-context-item"
        :disabled="!logContextMenu.lineText"
        @click="copyContextLine($event)"
      >{{ t('logPanel.copyLine') }}</button>
      <button
        type="button"
        class="log-context-item"
        :disabled="!logContextMenu.selectedText"
        @click="copyContextSelection($event)"
      >{{ t('logPanel.copySelection') }}</button>
      <div class="log-context-separator"></div>
      <button
        type="button"
        class="log-context-item"
        :disabled="totalLogCount === 0"
        @click="selectAllLogs($event)"
      >{{ t('logPanel.selectAllLogs') }}</button>
      <button
        type="button"
        class="log-context-item"
        :disabled="totalLogCount === 0"
        @click="copyAllLogs($event)"
      >{{ t('logPanel.copyAllLogs') }}</button>
    </div>

    <div
      v-if="logSuccessToast.visible"
      ref="logSuccessToastRef"
      class="log-success-toast"
      :style="{ left: `${logSuccessToast.x}px`, top: `${logSuccessToast.y}px` }"
      role="status"
    >
      {{ logSuccessToast.text }}
    </div>
  </div>
</template>

<style scoped>
.log-panel {
  background:
    linear-gradient(180deg, var(--app-surface), color-mix(in srgb, var(--app-surface) 88%, #000 2%));
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  position: relative;
}

.log-select-buffer {
  position: fixed;
  left: -9999px;
  top: 0;
  width: 1px;
  height: 1px;
  opacity: 0;
  pointer-events: none;
}

.log-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 7px 12px;
  font-size: 12px;
  font-weight: 600;
  color: var(--el-text-color-secondary);
  background:
    linear-gradient(180deg, var(--app-surface-soft), color-mix(in srgb, var(--app-surface-soft) 78%, var(--app-surface)));
  border-bottom: 1px solid var(--app-border-soft);
  user-select: none;
  flex-shrink: 0;
}

.log-title {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--app-text-primary);
}

.log-count {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: 8px;
  background: var(--el-color-primary);
  color: #fff;
  font-size: 10px;
  font-weight: 600;
}

.log-running-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 16px;
  padding: 0 5px;
  border-radius: 8px;
  background: var(--el-color-danger);
  color: #fff;
  font-size: 10px;
  font-weight: 600;
  animation: log-pulse 1.5s ease-in-out infinite;
}

@keyframes log-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.6; }
}

.log-header-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}

.log-progress-area {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 7px 12px;
  border-bottom: 1px solid var(--app-border-soft);
  background: linear-gradient(
    90deg,
    color-mix(in srgb, var(--app-accent-primary) 10%, var(--app-surface-soft)),
    color-mix(in srgb, var(--app-accent-secondary) 9%, var(--app-surface-soft))
  );
  flex-shrink: 0;
}

.log-progress-item {
  display: grid;
  grid-template-columns: minmax(140px, 1.1fr) minmax(96px, max-content) minmax(220px, 1fr);
  align-items: center;
  gap: 10px;
  width: 100%;
  font-size: 11px;
}

.log-progress-label {
  color: var(--el-text-color-secondary);
  font-weight: 500;
  display: block;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.log-progress-eta {
  color: var(--el-color-warning);
  font-size: 10px;
  display: block;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.log-progress-eta-empty {
  visibility: hidden;
}

.log-progress-meter {
  display: grid;
  grid-template-columns: minmax(80px, 1fr) max-content;
  align-items: center;
  gap: 8px;
  min-width: 0;
  width: 100%;
  overflow: visible;
}

.log-progress-bar-track {
  min-width: 0;
  width: 100%;
  height: 9px;
  background: color-mix(in srgb, var(--app-accent-deep) 12%, var(--app-surface));
  border-radius: 999px;
  overflow: hidden;
}

.log-progress-bar-fill {
  height: 100%;
  background: linear-gradient(90deg, var(--app-accent-primary), var(--app-accent-secondary));
  border-radius: 999px;
  transition: width 0.3s ease;
}

.log-progress-text {
  min-width: 8ch;
  width: max-content;
  max-width: 18ch;
  flex-shrink: 0;
  padding: 1px 6px;
  text-align: right;
  font-family: 'Consolas', 'Courier New', monospace;
  color: var(--app-text-primary);
  font-size: 10px;
  line-height: 1.4;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  border: 1px solid color-mix(in srgb, var(--app-border) 70%, transparent);
  border-radius: 4px;
  background: color-mix(in srgb, var(--app-surface) 86%, #fff 4%);
}

@media (max-width: 760px) {
  .log-progress-item {
    grid-template-columns: minmax(0, 1fr);
    gap: 5px;
  }

  .log-progress-eta-empty {
    display: none;
  }
}

.log-stop-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: none;
  border-radius: 4px;
  background: var(--el-color-danger);
  color: #fff;
  cursor: pointer;
  transition: opacity 0.15s, transform 0.1s;
  flex-shrink: 0;
}

.log-icon-btn {
  appearance: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 4px;
  background: transparent;
  color: var(--el-text-color-secondary);
  cursor: pointer;
  transition: background 0.15s, color 0.15s, transform 0.1s, opacity 0.15s;
  flex-shrink: 0;
}

.log-icon-btn:hover:not(:disabled) {
  background: var(--app-surface);
  color: var(--el-color-primary);
  transform: scale(1.08);
}

.log-icon-btn:active:not(:disabled) {
  transform: scale(0.94);
  opacity: 0.75;
}

.log-icon-btn:disabled {
  color: var(--el-text-color-disabled);
  cursor: default;
  opacity: 0.55;
}

.log-icon-loading {
  width: 13px;
  height: 13px;
  border: 2px solid color-mix(in srgb, var(--el-color-primary) 30%, transparent);
  border-top-color: var(--el-color-primary);
  border-radius: 50%;
  animation: log-spin 0.75s linear infinite;
}

@keyframes log-spin {
  to { transform: rotate(360deg); }
}

.log-stop-btn:hover {
  opacity: 0.85;
  transform: scale(1.08);
}

.log-stop-btn:active {
  transform: scale(0.94);
  opacity: 0.75;
}

.log-body {
  flex: 1;
  overflow-y: auto;
  padding: 5px 0;
  font-family: 'Consolas', 'Courier New', monospace;
  font-size: 11px;
  line-height: 1.5;
  background: color-mix(in srgb, var(--app-surface) 86%, #000);
}

.log-body-all-selected .log-line {
  background: color-mix(in srgb, var(--el-color-primary) 16%, transparent);
}

.log-body-all-selected .log-time,
.log-body-all-selected .log-msg {
  color: var(--el-color-primary);
}

.log-empty {
  margin: 10px;
  padding: 16px;
  text-align: center;
  color: var(--el-text-color-placeholder);
  font-size: 12px;
  border: 1px dashed var(--app-border-soft);
  border-radius: 10px;
  background: var(--app-surface-soft);
}

.log-line {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 1px 12px;
  transition: background 0.1s;
}

.log-line:hover {
  background: var(--app-surface-soft);
}

.log-time {
  color: var(--el-text-color-placeholder);
  flex-shrink: 0;
  font-size: 10px;
}

.log-msg {
  flex: 1;
  word-break: break-all;
}

.log-info .log-msg {
  color: var(--el-text-color-secondary);
}

.log-success .log-msg {
  color: var(--el-color-success);
}

.log-warn .log-msg {
  color: var(--el-color-warning);
}

.log-error .log-msg {
  color: var(--el-color-danger);
}

.log-context-menu {
  position: fixed;
  z-index: 3000;
  min-width: 172px;
  padding: 5px;
  border: 1px solid var(--app-border);
  border-radius: 7px;
  background: var(--app-surface);
  box-shadow: var(--app-shadow-md);
}

.log-context-item {
  display: block;
  width: 100%;
  padding: 6px 10px;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: var(--el-text-color-primary);
  font-size: 12px;
  line-height: 1.3;
  text-align: left;
  cursor: pointer;
}

.log-context-item:hover:not(:disabled) {
  background: var(--app-surface-soft);
}

.log-context-item:disabled {
  color: var(--el-text-color-disabled);
  cursor: default;
}

.log-context-separator {
  height: 1px;
  margin: 4px 3px;
  background: var(--app-border-soft);
}

.log-success-toast {
  position: fixed;
  z-index: 10030;
  max-width: min(260px, calc(100vw - 16px));
  padding: 7px 11px;
  border: 1px solid var(--el-color-success-light-5);
  border-radius: 7px;
  background: var(--el-color-success-light-9);
  color: var(--el-color-success);
  box-shadow: var(--app-shadow-md);
  font-size: 12px;
  line-height: 1.35;
  pointer-events: none;
  white-space: nowrap;
}
</style>
