/**
 * WorkspaceStoreUtils - lazy singleton wrapper for Tauri settings store.
 */

import { load } from '@tauri-apps/plugin-store'

/** Store helper utility class for the WorkSpace module. */
export class WorkspaceStoreUtils {
  /** Store instance cache (lazy-initialized). */
  private static storePromise: ReturnType<typeof load> | null = null

  /** Get Tauri persistent store instance. */
  static async getStore(): Promise<ReturnType<typeof load>> {
    if (!WorkspaceStoreUtils.storePromise) {
      WorkspaceStoreUtils.storePromise = load('settings.json', { defaults: {}, autoSave: true })
    }
    return WorkspaceStoreUtils.storePromise
  }
}
