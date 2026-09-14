import { appCacheDir } from '@tauri-apps/api/path'
import { load } from '@tauri-apps/plugin-store'

export const ASSET_MAP_CACHE_ROOT_KEY = 'asset-map-cache-root'

export class AssetMapCacheUtils {
  static async getCacheRoot(): Promise<string> {
    try {
      const store = await load('settings.json', { defaults: {}, autoSave: true })
      const saved = await store.get(ASSET_MAP_CACHE_ROOT_KEY)
      if (typeof saved === 'string' && saved.trim()) {
        return saved
      }
    } catch (_) {
      // Fall back to the app cache directory when settings cannot be loaded.
    }

    return await AssetMapCacheUtils.getDefaultCacheRoot()
  }

  static async getDefaultCacheRoot(): Promise<string> {
    const root = await appCacheDir()
    return `${root.replace(/[\\/]$/, '')}${AssetMapCacheUtils.pathSeparator(root)}asset-map`
  }

  private static pathSeparator(path: string): string {
    return path.includes('\\') ? '\\' : '/'
  }
}
