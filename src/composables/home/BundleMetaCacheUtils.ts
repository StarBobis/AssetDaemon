/**
 * BundleMetaCacheUtils - WorkSpace BundleMeta LRU cache helper.
 *
 * The helper owns cache size accounting and eviction logic so useWorkspace can focus on
 * page state and user actions. All methods are static to follow this project's utility
 * class convention for reusable TypeScript logic.
 */

import type { AssetSummary, BundleMeta } from '../../types'

/** Cache statistics shown in the UI/log output. */
export interface BundleMetaCacheStats {
  entries: number
  usedBytes: number
  maxBytes: number
  usedGB: string
  maxGB: string
}

/** Bundle metadata cache utility with LRU behavior. */
export class BundleMetaCacheUtils {
  /**
   * Get a cached BundleMeta and mark it as recently used.
   *
   * JavaScript Map keeps insertion order, so delete + set moves the hit to the end.
   */
  static getCachedBundleMeta(
    bundleMetaCache: Map<string, BundleMeta>,
    path: string,
  ): BundleMeta | undefined {
    const meta = bundleMetaCache.get(path)
    if (meta) {
      bundleMetaCache.delete(path)
      bundleMetaCache.set(path, meta)
    }
    return meta
  }

  /** Collect assets from every already-loaded cache entry for the All Assets panel. */
  static getAllCachedAssets(bundleMetaCache: Map<string, BundleMeta>): AssetSummary[] {
    const allAssets: AssetSummary[] = []
    for (const [bundlePath, meta] of bundleMetaCache.entries()) {
      if (meta.assets) {
        for (const asset of meta.assets) {
          allAssets.push({
            ...asset,
            source_bundle_path: bundlePath,
          })
        }
      }
    }
    return allAssets
  }

  /** Roughly estimate BundleMeta memory cost for LRU eviction decisions. */
  static estimateBundleMetaSize(meta: BundleMeta): number {
    let totalBytes = 0

    if (meta.assets) {
      for (const asset of meta.assets) {
        totalBytes += asset.name.length * 2 + asset.path.length * 2 + asset.class_name.length * 2 + 32
      }
    }

    if (meta.parse_logs) {
      for (const log of meta.parse_logs) {
        totalBytes += log.length * 2
      }
    }

    if (meta.nodes) {
      for (const node of meta.nodes) {
        totalBytes += node.name.length * 2 + 16
      }
    }

    if (meta.files_diag) {
      for (const diagnostic of meta.files_diag) {
        totalBytes += diagnostic.name.length * 2 + 32
      }
    }

    return totalBytes + 1024
  }

  /** Store one BundleMeta entry and evict oldest entries until it fits the configured limit. */
  static setCachedBundleMeta(
    bundleMetaCache: Map<string, BundleMeta>,
    path: string,
    meta: BundleMeta,
    currentUsedBytes: number,
    maxBytes: number,
  ): number {
    let nextUsedBytes = currentUsedBytes
    const oldMeta = bundleMetaCache.get(path)

    if (oldMeta) {
      nextUsedBytes -= BundleMetaCacheUtils.estimateBundleMetaSize(oldMeta)
      bundleMetaCache.delete(path)
    }

    const entryBytes = BundleMetaCacheUtils.estimateBundleMetaSize(meta)
    if (entryBytes > maxBytes) {
      return nextUsedBytes
    }

    while (nextUsedBytes + entryBytes > maxBytes && bundleMetaCache.size > 0) {
      const [firstKey] = bundleMetaCache.keys()
      const evictedMeta = bundleMetaCache.get(firstKey)
      if (!evictedMeta) break
      nextUsedBytes -= BundleMetaCacheUtils.estimateBundleMetaSize(evictedMeta)
      bundleMetaCache.delete(firstKey)
    }

    bundleMetaCache.set(path, meta)
    return nextUsedBytes + entryBytes
  }

  /** Build display-friendly cache statistics. */
  static getBundleCacheStats(
    bundleMetaCache: Map<string, BundleMeta>,
    usedBytes: number,
    maxBytes: number,
  ): BundleMetaCacheStats {
    return {
      entries: bundleMetaCache.size,
      usedBytes,
      maxBytes,
      usedGB: (usedBytes / (1024 * 1024 * 1024)).toFixed(2),
      maxGB: (maxBytes / (1024 * 1024 * 1024)).toFixed(0),
    }
  }

  /** Evict oldest entries until the cache obeys the new size limit. */
  static shrinkToLimit(
    bundleMetaCache: Map<string, BundleMeta>,
    currentUsedBytes: number,
    maxBytes: number,
  ): number {
    let nextUsedBytes = currentUsedBytes

    while (nextUsedBytes > maxBytes && bundleMetaCache.size > 0) {
      const [firstKey] = bundleMetaCache.keys()
      const evictedMeta = bundleMetaCache.get(firstKey)
      if (!evictedMeta) break
      nextUsedBytes -= BundleMetaCacheUtils.estimateBundleMetaSize(evictedMeta)
      bundleMetaCache.delete(firstKey)
    }

    return nextUsedBytes
  }
}
