import { appDataDir, join } from '@tauri-apps/api/path'
import { mkdir, writeTextFile } from '@tauri-apps/plugin-fs'
import { revealItemInDir } from '@tauri-apps/plugin-opener'

export interface LogExportEntry {
  time: string
  message: string
  type: 'info' | 'success' | 'warn' | 'error'
}

export class LogUtils {
  static formatLogEntry(entry: LogExportEntry): string {
    return `[${entry.time}] [${entry.type.toUpperCase()}] ${entry.message}`
  }

  static buildTimestampedLogFileName(date = new Date()): string {
    const pad = (value: number) => String(value).padStart(2, '0')
    const yyyy = date.getFullYear()
    const mm = pad(date.getMonth() + 1)
    const dd = pad(date.getDate())
    const hh = pad(date.getHours())
    const min = pad(date.getMinutes())
    return `assetdaemon-log-${yyyy}${mm}${dd}-${hh}${min}.txt`
  }

  static buildLogText(entries: LogExportEntry[]): string {
    return entries.map(LogUtils.formatLogEntry).join('\n') + '\n'
  }

  static async exportLogHistory(entries: LogExportEntry[]): Promise<string | null> {
    if (entries.length === 0) return null

    const appDataPath = await appDataDir()
    const logsDir = await join(appDataPath, 'logs')
    const filePath = await join(logsDir, LogUtils.buildTimestampedLogFileName())

    await mkdir(logsDir, { recursive: true })
    await writeTextFile(filePath, LogUtils.buildLogText(entries))
    await revealItemInDir(filePath)

    return filePath
  }
}
