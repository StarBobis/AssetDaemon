import type { AssetSummary } from '../types'

export const LIKELY_EMPTY_MESH_PREVIEW_ERROR = '__LIKELY_EMPTY_MESH__'

export class AssetClassificationUtils {
  static isLikelyEmptyMesh(asset: AssetSummary): boolean {
    const byteSize = Number(asset.byte_size)
    return String(asset.class_name).toLowerCase() === 'mesh' &&
      Number.isFinite(byteSize) &&
      byteSize > 0 &&
      byteSize <= 512
  }
}
