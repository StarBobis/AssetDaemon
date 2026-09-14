import { describe, expect, it, vi } from 'vitest'
import { WorkspaceAssetMapClient } from './WorkspaceAssetMapClient'

describe('WorkspaceAssetMapClient', () => {
  it('builds a stable paged BundleMeta scaffold', () => {
    const client = new WorkspaceAssetMapClient({
      invoke: vi.fn(),
      getCacheRoot: async () => 'cache-root',
    })

    expect(client.createPagedBundleMeta('bundle-a')).toEqual({
      path: 'bundle-a',
      size: 0,
      compressed: false,
      unity_version: '',
      nodes: [],
      assets: [],
      files_diag: [],
      parse_logs: ['(Loaded from AssetMap index pages)'],
    })
  })

  it('hides Tauri query shape for paged bundle asset reads', async () => {
    const invoke = vi.fn().mockResolvedValue({
      assets: [],
      total: 0,
      offset: 0,
      limit: 200,
    })
    const client = new WorkspaceAssetMapClient({
      invoke,
      getCacheRoot: async () => 'cache-root',
    })

    await client.loadBundleAssetPage({
      workspaceDir: 'workspace',
      bundlePath: 'bundle-a',
      search: 'mesh',
      classNames: ['Mesh'],
      advancedFilter: {
        classNames: ['Mesh'],
        nameQuery: '',
        nameMatchMode: 'contains',
        nameIncludeQueries: ['body'],
        nameExcludeQuery: '',
        nameExcludeQueries: ['shadow'],
        bundleQuery: '',
        pathIdQuery: '42',
        minSizeText: '1',
        maxSizeText: '2',
        sizeUnit: 'KB',
        sortBy: 'size',
        sortDirection: 'desc',
      },
      offset: 40,
      limit: 20,
    })

    expect(invoke).toHaveBeenCalledWith('query_map_assets', {
      workspaceDir: 'workspace',
      assetMapCacheRoot: 'cache-root',
      options: {
        search: 'mesh',
        class_names: ['Mesh'],
        bundle_path: 'bundle-a',
        name_match_mode: 'contains',
        name_include_queries: ['body'],
        name_exclude_queries: ['shadow'],
        path_id_query: '42',
        min_size: 1024,
        max_size: 2048,
        offset: 40,
        limit: 20,
        sort_by: 'size',
        sort_direction: 'desc',
      },
    })
  })

  it('checks AssetMap availability before callers enter paged loading mode', async () => {
    const invoke = vi.fn().mockResolvedValue(null)
    const client = new WorkspaceAssetMapClient({
      invoke,
      getCacheRoot: async () => 'cache-root',
    })

    await expect(client.hasAssetMap('workspace')).resolves.toBe(false)
    expect(invoke).toHaveBeenCalledWith('get_map_summary', {
      workspaceDir: 'workspace',
      assetMapCacheRoot: 'cache-root',
    })
  })

  it('reports AssetMap availability when summary exists', async () => {
    const invoke = vi.fn().mockResolvedValue({
      built_at: 1,
      built_at_formatted: 'now',
      bundle_count: 1,
      asset_count: 2,
      parsed_count: 1,
      cancelled: false,
    })
    const client = new WorkspaceAssetMapClient({
      invoke,
      getCacheRoot: async () => 'cache-root',
    })

    await expect(client.hasAssetMap('workspace')).resolves.toBe(true)
  })

  it('appends later AssetMap pages without losing BundleMeta context', () => {
    const client = new WorkspaceAssetMapClient({
      invoke: vi.fn(),
      getCacheRoot: async () => 'cache-root',
    })
    const firstPage = client.applyBundleAssetPage(null, 'bundle-a', [
      {
        path: 'mesh-a',
        name: 'mesh-a',
        class_name: 'Mesh',
        class_id: 43,
        path_id: '1',
        byte_size: 10,
        source_bundle_path: 'bundle-a',
      },
    ], true)

    const secondPage = client.applyBundleAssetPage(firstPage, 'bundle-a', [
      {
        path: 'texture-a',
        name: 'texture-a',
        class_name: 'Texture2D',
        class_id: 28,
        path_id: '2',
        byte_size: 20,
        source_bundle_path: 'bundle-a',
      },
    ], false)

    expect(secondPage.path).toBe('bundle-a')
    expect(secondPage.parse_logs).toEqual(['(Loaded from AssetMap index pages)'])
    expect(secondPage.assets.map(asset => asset.path_id)).toEqual(['1', '2'])
  })

  it('resets previous AssetMap page results when filters change', () => {
    const client = new WorkspaceAssetMapClient({
      invoke: vi.fn(),
      getCacheRoot: async () => 'cache-root',
    })
    const oldMeta = client.applyBundleAssetPage(null, 'bundle-a', [
      {
        path: 'mesh-a',
        name: 'mesh-a',
        class_name: 'Mesh',
        class_id: 43,
        path_id: '1',
        byte_size: 10,
      },
    ], true)

    const filteredMeta = client.applyBundleAssetPage(oldMeta, 'bundle-a', [
      {
        path: 'texture-a',
        name: 'texture-a',
        class_name: 'Texture2D',
        class_id: 28,
        path_id: '2',
        byte_size: 20,
      },
    ], true)

    expect(filteredMeta.assets.map(asset => asset.path_id)).toEqual(['2'])
  })
})
