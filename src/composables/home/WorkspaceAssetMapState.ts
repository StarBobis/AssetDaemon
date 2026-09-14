import { ref } from 'vue'
import type { MapAssetClassStat } from '../../types'

export const ASSET_MAP_FULL_QUERY_LIMIT = 2_147_483_647

export function createWorkspaceAssetMapState() {
  return {
    assetMapVersion: ref(0),
    isBundleAssetMapPaged: ref(false),
    bundleAssetMapTotal: ref(0),
    bundleAssetClassStats: ref<MapAssetClassStat[]>([]),
    bundleAssetMapLoading: ref(false),
  }
}
