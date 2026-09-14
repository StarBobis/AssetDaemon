import { describe, expect, it } from 'vitest'
import { ref } from 'vue'
import { WorkspaceComputedUtils } from './WorkspaceComputedUtils'
import type { AssetSummary } from '../../types'

function createComputedForAsset(className: string) {
  const asset: AssetSummary = {
    path: className,
    name: className,
    class_name: className,
    class_id: 0,
    path_id: '1',
    byte_size: 1,
  }

  return WorkspaceComputedUtils.create({
    bundleFiles: ref([]),
    bundleMeta: ref(null),
    previewTargetAsset: ref(asset),
    highlightedFilePaths: ref([]),
    activeClassFilter: ref(''),
    activeAssetTypeFilters: ref([]),
    assetSearchQuery: ref(''),
    activeFilters: ref([]),
    fileSizeSortDirection: ref('none'),
    visibleCount: ref(50),
    visibleAssetCount: ref(200),
    allAssetTypes: [],
  })
}

describe('WorkspaceComputedUtils previewType', () => {
  it('routes GameObject assets to the mesh preview panel', () => {
    expect(createComputedForAsset('GameObject').previewType.value).toBe('mesh')
  })

  it('keeps typed preview for non-model structured assets', () => {
    expect(createComputedForAsset('Material').previewType.value).toBe('typed')
  })
})
