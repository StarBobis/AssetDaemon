import { describe, expect, it } from 'vitest'
import { AssetDisplayUtils } from './AssetDisplayUtils'
import type { AssetSummary } from '../types'

function asset(overrides: Partial<AssetSummary>): AssetSummary {
  return {
    path: '',
    name: '',
    class_name: 'Texture2D',
    class_id: 0,
    path_id: '1',
    byte_size: 0,
    ...overrides,
  }
}

describe('AssetDisplayUtils', () => {
  it('uses GameObject asset_path stem instead of unreadable indexed name', () => {
    expect(AssetDisplayUtils.getAssetDisplayName(asset({
      class_name: 'GameObject',
      name: '\u0000\u0000',
      path: 'assets/res/prefab/proxy_body/yaodaojibulletproxybody_amulet_self_level1.prefab',
    }))).toBe('yaodaojibulletproxybody_amulet_self_level1')
  })

  it('keeps readable Unity object names for non-GameObject assets', () => {
    expect(AssetDisplayUtils.getAssetDisplayName(asset({
      class_name: 'Mesh',
      name: 'hero_body_lod0',
      path: 'assets/res/model/hero_body_lod0.mesh',
    }))).toBe('hero_body_lod0')
  })

  it('uses bundle short name before path id for unnamed component assets', () => {
    expect(AssetDisplayUtils.getAssetDisplayName(asset({
      class_name: 'Animator',
      path_id: '2081448936342685611',
      source_bundle_path: 'C:\\Users\\Administrator\\Desktop\\NarakaDecrypt\\0\\0\\00027328d2eb5747',
    }))).toBe('Animator @ 00027328d2eb5747')
  })
})
