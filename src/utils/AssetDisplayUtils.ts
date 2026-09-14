import type { AssetSummary } from '../types'

export class AssetDisplayUtils {
  /** Keep binary-looking names out of visible labels and export defaults. */
  static isReadableAssetName(assetName: string): boolean {
    if (!assetName) return false
    if (assetName.includes('\uFFFD')) return false
    return !/[\u0000-\u0008\u000B\u000C\u000E-\u001F]/.test(assetName)
  }

  /** Prefer resource identity over raw GameObject m_Name in asset browser rows. */
  static getAssetDisplayName(asset: AssetSummary): string {
    if (asset.class_name === 'GameObject') {
      const pathStem = AssetDisplayUtils.pathStem(asset.path)
      if (pathStem) return pathStem
      if (AssetDisplayUtils.isReadableAssetName(asset.name)) {
        return asset.name
      }
    }
    if (AssetDisplayUtils.isReadableAssetName(asset.name)) {
      return asset.name
    }
    const pathStem = AssetDisplayUtils.pathStem(asset.path)
    if (pathStem) return pathStem
    const bundleStem = AssetDisplayUtils.pathStem(asset.source_bundle_path || '')
    if (bundleStem) return `${asset.class_name} @ ${bundleStem}`
    return `${asset.class_name}_${asset.path_id}`
  }

  static getAssetExportName(asset: AssetSummary): string {
    return AssetDisplayUtils.getAssetDisplayName(asset)
  }

  private static pathStem(assetPath: string): string {
    if (!assetPath) return ''
    const fileName = assetPath.replace(/\\/g, '/').split('/').pop() || ''
    const dotIndex = fileName.lastIndexOf('.')
    if (dotIndex <= 0) return fileName
    return fileName.slice(0, dotIndex)
  }
}
