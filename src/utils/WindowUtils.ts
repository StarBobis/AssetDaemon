/**
 * Window utility class - provides Tauri window size persistence.
 *
 * Follows Soul.md convention: all methods must be class methods of a utility class, no free functions.
 * Low-level independent utility methods are categorized under src/utils/ in corresponding XXXUtils.ts files.
 *
 * Features:
 *   1. Restore last window size from store on startup
 *   2. Auto-detect current monitor size, clamp window to 85% of available area
 *   3. Debounce-save window size to store on resize
 *   4. Skip saving when maximized (to prevent oversized window on next launch)
 */

import { getCurrentWindow, LogicalSize, currentMonitor } from '@tauri-apps/api/window'
import { load } from '@tauri-apps/plugin-store'

type WindowBounds = {
  width: number
  height: number
}

type MaybePhysicalSize = WindowBounds & {
  toLogical?: (scaleFactor: number) => WindowBounds
}

/**
 * Window utility class.
 *
 * Encapsulates Tauri window size loading and persistent saving logic.
 * All methods are static methods, with no business logic - pure utility methods.
 */
export class WindowUtils {
  /** Debounce save window size timer handle */
  private static saveWindowTimer: ReturnType<typeof setTimeout> | null = null
  private static store_promise: ReturnType<typeof load> | null = null
  private static resize_unlisten: (() => void) | null = null

  static async initializeWindowPersistence(): Promise<void> {
    await WindowUtils.loadWindowBounds()

    if (WindowUtils.resize_unlisten) return

    try {
      const appWindow = getCurrentWindow()
      WindowUtils.resize_unlisten = await appWindow.listen('tauri://resize', () => {
        void WindowUtils.saveWindowBounds()
      })
    } catch (error) {
      console.error('Failed to initialize window size persistence:', error)
    }
  }

  /**
   * Load previously saved window size from store and restore it.
   *
   * Also detects current monitor size and clamps window to 85% of available area,
   * preventing oversized windows on next launch if saved while maximized.
   *
   * Important: monitor.size is PhysicalSize (physical pixels), must be converted
   * to LogicalSize using scaleFactor before comparing with saved logical pixel size.
   */
  static async loadWindowBounds(): Promise<void> {
    try {
      const s = await WindowUtils.getStore()
      const bounds = await s.get('window-bounds')
      if (bounds && typeof bounds === 'object') {
        const savedBounds = WindowUtils.normalizeBounds(bounds)
        if (savedBounds) {
          const appWindow = getCurrentWindow()
          const maxSize = await WindowUtils.getMaxLogicalWindowSize()

          const clampedWidth = WindowUtils.clampDimension(savedBounds.width, 800, maxSize.width)
          const clampedHeight = WindowUtils.clampDimension(savedBounds.height, 600, maxSize.height)

          await appWindow.setSize(new LogicalSize(clampedWidth, clampedHeight))
          await appWindow.center()
        }
      }
    } catch (e) {
      console.error('Failed to load window size:', e)
    }
  }

  /**
   * Save current window size to store.
   *
   * Called on window resize event (debounced at 300ms).
   * Skips saving if maximized, to avoid restoring fullscreen size on next launch.
   */
  static async saveWindowBounds(options?: { immediate?: boolean }): Promise<void> {
    if (WindowUtils.saveWindowTimer) {
      clearTimeout(WindowUtils.saveWindowTimer)
      WindowUtils.saveWindowTimer = null
    }

    if (options?.immediate) {
      await WindowUtils.writeCurrentWindowBounds()
      return
    }

    WindowUtils.saveWindowTimer = setTimeout(async () => {
      await WindowUtils.writeCurrentWindowBounds()
    }, 300)
  }

  private static getStore(): ReturnType<typeof load> {
    if (!WindowUtils.store_promise) {
      WindowUtils.store_promise = load('settings.json', { defaults: {}, autoSave: true })
    }

    return WindowUtils.store_promise
  }

  private static normalizeBounds(value: unknown): WindowBounds | null {
    if (!value || typeof value !== 'object') return null

    const bounds = value as Partial<WindowBounds>
    if (typeof bounds.width !== 'number' || typeof bounds.height !== 'number') return null
    if (!Number.isFinite(bounds.width) || !Number.isFinite(bounds.height)) return null
    if (bounds.width <= 0 || bounds.height <= 0) return null

    return {
      width: Math.round(bounds.width),
      height: Math.round(bounds.height),
    }
  }

  private static async getMaxLogicalWindowSize(): Promise<WindowBounds> {
    try {
      const monitor = await currentMonitor()
      if (monitor) {
        const logicalSize = monitor.size.toLogical(monitor.scaleFactor)
        return {
          width: Math.round(logicalSize.width * 0.85),
          height: Math.round(logicalSize.height * 0.85),
        }
      }
    } catch (error) {
      console.warn('Failed to get monitor info, using default upper limit:', error)
    }

    return { width: 1400, height: 900 }
  }

  private static async getCurrentLogicalWindowSize(): Promise<WindowBounds> {
    const appWindow = getCurrentWindow()
    const size = await appWindow.outerSize() as MaybePhysicalSize

    try {
      const monitor = await currentMonitor()
      if (monitor && typeof size.toLogical === 'function') {
        const logicalSize = size.toLogical(monitor.scaleFactor)
        return {
          width: Math.round(logicalSize.width),
          height: Math.round(logicalSize.height),
        }
      }
    } catch (error) {
      console.warn('Failed to convert window size to logical pixels:', error)
    }

    return {
      width: Math.round(size.width),
      height: Math.round(size.height),
    }
  }

  private static clampDimension(value: number, min: number, max: number): number {
    return Math.max(min, Math.min(Math.round(value), max))
  }

  private static async writeCurrentWindowBounds(): Promise<void> {
    try {
      const appWindow = getCurrentWindow()

      const maximized = await appWindow.isMaximized()
      if (maximized) return

      const size = await WindowUtils.getCurrentLogicalWindowSize()
      const s = await WindowUtils.getStore()
      await s.set('window-bounds', size)
      await s.save()
    } catch (error) {
      console.error('Failed to save window size:', error)
    }
  }
}
