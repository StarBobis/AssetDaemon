import { ref } from 'vue'
import type { AssetSummary, PreviewAnimationClipRef, PreviewAnimatorRef } from '../../types'
import type { MeshGeometryPreviewData, TexturePreviewCacheEntry } from './WorkspacePreviewUtils'

export const MESH_PREVIEW_CACHE_LIMIT = 96
export const TEXTURE_PREVIEW_CACHE_LIMIT = 192

export function createWorkspacePreviewState() {
  return {
    selectedAsset: ref<AssetSummary | null>(null),
    previewTargetAsset: ref<AssetSummary | null>(null),
    dumpTargetAsset: ref<AssetSummary | null>(null),
    assetProperties: ref<Record<string, string>>({}),
    previewFilePath: ref(''),
    meshGeometryData: ref<MeshGeometryPreviewData | null>(null),
    meshCache: new Map<string, MeshGeometryPreviewData | null>(),
    textureCache: new Map<string, TexturePreviewCacheEntry | null>(),
    textureInfo: ref<TexturePreviewCacheEntry | null>(null),
    typedPreviewData: ref<import('../../types').AssetTypedPreviewResult | null>(null),
    typedPreviewCache: new Map<string, import('../../types').AssetTypedPreviewResult | null>(),
    previewAnimators: ref<PreviewAnimatorRef[]>([]),
    previewAnimatorCache: new Map<string, PreviewAnimatorRef[]>(),
    previewAnimationClips: ref<PreviewAnimationClipRef[]>([]),
    previewAnimationClipCache: new Map<string, PreviewAnimationClipRef[]>(),
    selectedPreviewAnimationClip: ref<PreviewAnimationClipRef | null>(null),
    selectedPreviewMeshScope: ref<{ bundle_path: string; path_id: string; class_name: string; name?: string } | null>(null),
    textureByteSizeCache: new Map<string, number>(),
    previewLoading: ref(false),
    previewError: ref(''),
    rightTab: ref<'preview' | 'dump'>('preview'),
    dumpData: ref<import('../../types').DumpResult | null>(null),
    dumpLoading: ref(false),
    dumpError: ref(''),
    dumpCache: new Map<string, import('../../types').DumpResult | null>(),
    cacheDir: ref(''),
    contextMenuVisible: ref(false),
    contextMenuX: ref(0),
    contextMenuY: ref(0),
    contextMenuAsset: ref<AssetSummary | null>(null),
    dialogAsset: ref<AssetSummary | null>(null),
    showPropsDialog: ref(false),
  }
}
