import { describe, expect, it } from 'vitest'
import { WorkspacePreviewUtils } from './WorkspacePreviewUtils'

describe('WorkspacePreviewUtils cache keys', () => {
  it('keeps decoded texture byte sizes scoped by bundle path', () => {
    const cache = new Map<string, number>([
      [WorkspacePreviewUtils.getTextureAssetCacheKey('bundle-a', '42'), 1024],
      [WorkspacePreviewUtils.getTextureAssetCacheKey('bundle-b', '42'), 2048],
    ])

    expect(WorkspacePreviewUtils.getAssetSize({
      class_name: 'Texture2D',
      path_id: '42',
      byte_size: 512,
      source_bundle_path: 'bundle-a',
    }, cache)).toBe('1.0 KB')

    expect(WorkspacePreviewUtils.getAssetSize({
      class_name: 'Texture2D',
      path_id: '42',
      byte_size: 512,
      source_bundle_path: 'bundle-b',
    }, cache)).toBe('2.0 KB')
  })

  it('separates preview images by max edge while sharing the asset identity key', () => {
    expect(WorkspacePreviewUtils.getTextureAssetCacheKey('bundle-a', '42')).toBe('bundle-a::42')
    expect(WorkspacePreviewUtils.getTexturePreviewCacheKey('bundle-a', '42', 512)).toBe('bundle-a::42::edge=512')
    expect(WorkspacePreviewUtils.getTexturePreviewCacheKey('bundle-a', '42', 2048)).toBe('bundle-a::42::edge=2048')
  })
})

describe('WorkspacePreviewUtils mesh error classification', () => {
  it('recognizes English and normal Chinese likely-empty mesh diagnostics', () => {
    expect(WorkspacePreviewUtils.isLikelyEmptyMeshError('likely empty mesh: no vertices')).toBe(true)
    expect(WorkspacePreviewUtils.isLikelyEmptyMeshError('疑似空 Mesh：没有顶点')).toBe(true)
  })
})
