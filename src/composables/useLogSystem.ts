/**
 * useLogSystem - Log system composable
 *
 * Provides globally shared log state and methods. All child components
 * share the same reactive state by calling the same composable function
 * (module-level singleton pattern).
 *
 * Usage:
 *   const { logs, addLog, ... } = useLogSystem()
 */

import { computed, ref, reactive, nextTick } from 'vue'
import { invoke, Channel } from '@tauri-apps/api/core'
import { i18n } from '../i18n'
import { LogUtils } from '../utils/LogUtils'

export interface LogEntry {
  time: string
  message: string
  type: 'info' | 'success' | 'warn' | 'error'
}
export interface TaskLogEntry {
  task_id: string
  task_label: string
  level: LogEntry['type']
  message: string
  timestamp_ms: number
  progress_step: string | null
  progress_current: number | null
  progress_total: number | null
}

type ProgressTaskLogEntry = TaskLogEntry & {
  progress_step: string
  progress_current: number
  progress_total: number
}

export interface TaskProgress {
  progress_key: string
  task_id: string
  task_label: string
  step: string
  current: number
  total: number
  remaining_label?: string
}

export interface RunningTask {
  task_id: string
  task_label: string
  started_at_ms: number
}

// ============================================================
// Module-level state (singleton)
// Ref defined at module scope, all components calling useLogSystem() share the same state.
// ============================================================

/** Log list (keeps last 500 entries) */
const logs = ref<LogEntry[]>([])

/** Full in-memory log history for count and export. */
const logHistory = ref<LogEntry[]>([])

/** Total logs recorded in current session. */
const totalLogCount = computed(() => logHistory.value.length)

/** Log container DOM reference, used for auto-scroll to bottom */
const logContainerRef = ref<HTMLElement | null>(null)

/** Whether backend log channel has been registered (prevents duplicate registration) */
let backendChannelRegistered = false

/** Number of running background tasks (visual indicator for stop button) */
const runningTaskCount = ref(0)

/** Incremented after a successful stop request so task-specific UI can clear local loading states. */
const stopSignal = ref(0)

/** Task progress snapshot map: task_id -> TaskProgress (for progress bar display, deduplicated keeping latest) */
const taskProgressMap = reactive<Record<string, TaskProgress>>({})

/** Running task runtime map: task_id -> task label + frontend-observed start time */
const runningTaskMap = reactive<Record<string, RunningTask>>({})

/** Last visible progress log state, used to avoid flooding the log body with high-frequency progress ticks. */
const visibleProgressLogMap = new Map<string, { current: number; timestamp_ms: number }>()

/** Progress start snapshot for rough ETA estimation: progressKey -> first seen time/current. */
const progressStartMap = new Map<string, { started_at_ms: number; started_current: number }>()

function clearTaskProgress(taskId: string) {
  Object.keys(taskProgressMap)
    .filter(k => taskProgressMap[k].task_id === taskId)
    .forEach(k => {
      delete taskProgressMap[k]
      visibleProgressLogMap.delete(k)
      progressStartMap.delete(k)
    })
}

function clearAllTaskProgress() {
  Object.keys(taskProgressMap).forEach(k => delete taskProgressMap[k])
  visibleProgressLogMap.clear()
  progressStartMap.clear()
}

function shouldDisplayProgressEntry(entry: TaskLogEntry): entry is ProgressTaskLogEntry {
  if (entry.progress_current === null || entry.progress_total === null || entry.progress_step === null) {
    return false
  }

  return entry.task_label !== 'Batch Export' && !entry.task_id.startsWith('export_')
}

// ============================================================
// Export composable
// ============================================================

export function useLogSystem() {
  function appendLogEntry(entry: LogEntry) {
    logHistory.value.push(entry)
    logs.value.push(entry)
    if (logs.value.length > 500) {
      logs.value.splice(0, logs.value.length - 500)
    }
  }

  function scrollLogToBottom() {
    nextTick(() => {
      const el = logContainerRef.value
      if (el) el.scrollTop = el.scrollHeight
    })
  }

  function clearLogs() {
    logs.value = []
    logHistory.value = []
  }

  function buildAllLogText(): string {
    return LogUtils.buildLogText(logHistory.value)
  }

  async function exportLogs(): Promise<string | null> {
    return LogUtils.exportLogHistory(logHistory.value)
  }

  /**
   * Add a log entry with timestamp.
   *
   * @param message - Log text content
   * @param type - Log type, determines color style (info / success / warn / error)
   */
  function addLog(message: string, type: LogEntry['type'] = 'info') {
    const now = new Date()
    const time = now.toLocaleTimeString('en-US', { hour12: false })
    appendLogEntry({ time, message, type })
    scrollLogToBottom()
  }

  /**
   * Batch add logs (triggers only one reactive update).
   *
   * @param entries - Array of log entries [{ message, type }, ...]
   */
  function addLogBatch(entries: { message: string; type?: LogEntry['type'] }[]) {
    if (entries.length === 0) return
    const now = new Date()
    const time = now.toLocaleTimeString('en-US', { hour12: false })
    for (const entry of entries) {
      appendLogEntry({ time, message: entry.message, type: entry.type || 'info' })
    }
    scrollLogToBottom()
  }

  function shouldAppendBackendEntry(entry: TaskLogEntry): boolean {
    if (entry.progress_current === null || entry.progress_total === null || entry.progress_step === null) {
      return true
    }
    if (entry.level !== 'info') return true

    const progressKey = `${entry.task_id}:${entry.progress_step}`
    const current = entry.progress_current
    const total = entry.progress_total
    if (current <= 1 || current >= total) {
      visibleProgressLogMap.set(progressKey, { current, timestamp_ms: entry.timestamp_ms })
      return true
    }

    const previous = visibleProgressLogMap.get(progressKey)
    const minStep = Math.max(1, Math.floor(total / 100))
    const elapsedMs = previous ? entry.timestamp_ms - previous.timestamp_ms : Number.POSITIVE_INFINITY
    const advanced = previous ? current - previous.current : minStep
    if (!previous || advanced >= minStep || elapsedMs >= 5000) {
      visibleProgressLogMap.set(progressKey, { current, timestamp_ms: entry.timestamp_ms })
      return true
    }

    return false
  }

  /**
   * Register backend log channel.
   *
   * Creates a persistent Tauri IPC Channel and registers it with the Rust backend TaskLogger.
   * After this, all background task logs are pushed to the frontend log panel through this channel.
   * Should be called once in WorkSpace page's onMounted.
   * Built-in duplicate registration protection.
   */
  async function registerBackendLogChannel() {
    if (backendChannelRegistered) return
    backendChannelRegistered = true

    try {
      const channel = new Channel<TaskLogEntry>()
      channel.onmessage = (entry: TaskLogEntry) => {
        if (entry.task_id && entry.level !== 'success' && entry.level !== 'error') {
          runningTaskMap[entry.task_id] = {
            task_id: entry.task_id,
            task_label: entry.task_label || runningTaskMap[entry.task_id]?.task_label || entry.task_id,
            started_at_ms: runningTaskMap[entry.task_id]?.started_at_ms || entry.timestamp_ms || Date.now(),
          }
        }

        // Update progress bar data (deduplicate: keep only latest per task phase)
        if (shouldDisplayProgressEntry(entry)) {
          const progressKey = `${entry.task_id}:${entry.progress_step}`
          const progressStart = progressStartMap.get(progressKey) || {
            started_at_ms: entry.timestamp_ms,
            started_current: entry.progress_current,
          }
          progressStartMap.set(progressKey, progressStart)
          taskProgressMap[progressKey] = {
            progress_key: progressKey,
            task_id: entry.task_id,
            task_label: entry.task_label,
            step: entry.progress_step,
            current: entry.progress_current,
            total: entry.progress_total,
            remaining_label: estimateRemainingLabel(
              progressStart.started_at_ms,
              progressStart.started_current,
              entry.timestamp_ms,
              entry.progress_current,
              entry.progress_total,
            ),
          }
        }
        const taskEnded =
          entry.level === 'success' ||
          entry.level === 'error' ||
          (entry.level === 'warn' && /cancelled|canceled|停止|取消/i.test(entry.message))

        // Clear progress bar when task ends.
        if (taskEnded) {
          clearTaskProgress(entry.task_id)
          delete runningTaskMap[entry.task_id]
        }

        // Convert backend log to frontend LogEntry and append to log panel
        if (shouldAppendBackendEntry(entry)) {
          const date = new Date(entry.timestamp_ms)
          const time = date.toLocaleTimeString('en-US', { hour12: false })
          const label = entry.task_label ? `[${entry.task_label}] ` : ''
          const message = `${label}${entry.message}`
          appendLogEntry({ time, message, type: entry.level })
          scrollLogToBottom()
        }
      }

      await invoke('register_log_channel', { channel })
      // Don't repeat notification in frontend log, backend already sent confirmation via TaskLogger
    } catch (e) {
      console.error('Failed to register backend log channel:', e)
      backendChannelRegistered = false
    }
  }

  /**
   * Stop all running background tasks.
   *
   * Calls the Rust backend's stop_all_tasks command,
   * setting all registered task cancellation tokens to true.
   * Each task stops at the next checkpoint when it detects cancellation.
   *
   * @returns The number of tasks cancelled
   */
  async function stopAllTasks(): Promise<number> {
    try {
      const count = await invoke<number>('stop_all_tasks')
      if (count > 0) {
        addLog(i18n.global.t('logPanel.stopRequested', { count }), 'warn')
        // Clear all progress bars immediately for instant visual feedback
        clearAllTaskProgress()
        Object.keys(runningTaskMap).forEach(k => delete runningTaskMap[k])
        runningTaskCount.value = 0
        stopSignal.value++
      } else {
        addLog(i18n.global.t('logPanel.noRunningTasks'), 'info')
      }
      return count
    } catch (e) {
      addLog(i18n.global.t('logPanel.stopFailed', { error: e }), 'error')
      return 0
    }
  }

  /**
   * Refresh the count of running tasks.
   *
   * Queries the backend for the current active task count using get_running_tasks command.
   */
  async function refreshRunningTaskCount() {
    try {
      const tasks = await invoke<string[]>('get_running_tasks')
      runningTaskCount.value = tasks.length
      const activeIds = new Set(tasks)
      const now = Date.now()
      for (const taskId of tasks) {
        if (!runningTaskMap[taskId]) {
          runningTaskMap[taskId] = {
            task_id: taskId,
            task_label: taskId,
            started_at_ms: now,
          }
        }
      }
      Object.keys(runningTaskMap)
        .filter(taskId => !activeIds.has(taskId))
        .forEach(taskId => {
          clearTaskProgress(taskId)
          delete runningTaskMap[taskId]
        })
      Object.keys(taskProgressMap)
        .filter(progressKey => !activeIds.has(taskProgressMap[progressKey].task_id))
        .forEach(progressKey => {
          visibleProgressLogMap.delete(progressKey)
          progressStartMap.delete(progressKey)
          delete taskProgressMap[progressKey]
        })
    } catch {
      runningTaskCount.value = 0
      Object.keys(runningTaskMap).forEach(k => delete runningTaskMap[k])
      clearAllTaskProgress()
    }
  }

  function estimateRemainingLabel(
    startedAtMs: number,
    startedCurrent: number,
    currentTimestampMs: number,
    current: number,
    total: number,
  ): string | undefined {
    if (total <= 0 || current <= 0 || current >= total) return undefined
    const elapsedSeconds = Math.max(1, (currentTimestampMs - startedAtMs) / 1000)
    const progressDelta = current - startedCurrent
    const effectiveProgress = progressDelta > 0 ? progressDelta : current
    if (effectiveProgress <= 0) return undefined

    const speed = effectiveProgress / elapsedSeconds
    if (speed <= 0) return undefined

    const remainingSeconds = Math.ceil((total - current) / speed)
    return formatDurationLabel(remainingSeconds)
  }

  function formatDurationLabel(totalSeconds: number): string {
    const seconds = Math.max(0, Math.floor(totalSeconds))
    const hours = Math.floor(seconds / 3600)
    const minutes = Math.floor((seconds % 3600) / 60)
    const restSeconds = seconds % 60
    if (hours > 0) {
      return `${hours}h ${String(minutes).padStart(2, '0')}m ${String(restSeconds).padStart(2, '0')}s`
    }
    if (minutes > 0) {
      return `${minutes}m ${String(restSeconds).padStart(2, '0')}s`
    }
    return `${restSeconds}s`
  }

  return {
    /** Log entry list */
    logs,
    /** Full log history for count and export */
    logHistory,
    /** Total logs recorded in current session */
    totalLogCount,
    /** Log container DOM reference (for auto-scroll to bottom) */
    logContainerRef,
    /** Number of running background tasks */
    runningTaskCount,
    /** Global task stop signal */
    stopSignal,
    /** Task progress snapshot map (for progress bar) */
    taskProgressMap,
    /** Running task runtime snapshot map */
    runningTaskMap,
    /** Add a log entry */
    addLog,
    /** Batch add logs (triggers only one reactive update) */
    addLogBatch,
    /** Clear visible logs and export history */
    clearLogs,
    /** Build copy/export text from full log history */
    buildAllLogText,
    /** Export all historical logs to AppData/logs and reveal the file */
    exportLogs,
    /** Register backend log channel (call once on page init) */
    registerBackendLogChannel,
    /** Stop all running background tasks */
    stopAllTasks,
    /** Refresh running task count */
    refreshRunningTaskCount,
  }
}

