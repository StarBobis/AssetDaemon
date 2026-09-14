import type { ExportFormat } from '../types'

/** Default export formats per Unity class; first item is the recommended default. */
export const DEFAULT_EXPORT_FORMATS: Record<string, ExportFormat[]> = {
  Texture2D: ['dds', 'png', 'tga'],
  Sprite: ['png'],
  SpriteMask: ['png'],
  Mesh: ['glb', 'obj'],
  GameObject: ['glb', 'jsontree', 'raw'],
  AudioClip: ['raw'],
  TextAsset: ['txt', 'json', 'bytes'],
  Font: ['ttf', 'otf'],
  Shader: ['shader', 'bin'],
  MonoBehaviour: ['jsontree'],
  AnimationClip: ['jsontree', 'raw'],
  Animator: ['glb', 'jsontree', 'raw'],
  Avatar: ['jsontree', 'raw'],
  RuntimeAnimatorController: ['jsontree', 'raw'],
  AnimatorController: ['jsontree', 'raw'],
  AnimatorOverrideController: ['jsontree', 'raw'],
  VideoClip: ['ogv', 'binraw'],
  MovieTexture: ['ogv', 'binraw'],
}

export const EXPORT_FORMAT_LABELS: Record<string, string> = {
  dds: 'DDS (Native Lossless)',
  png: 'PNG (Cropped Preview)',
  tga: 'TGA',
  glb: 'GLB',

  obj: 'OBJ',
  wav: 'WAV',
  txt: 'Text',
  json: 'JSON',
  bytes: 'Bytes',
  ttf: 'TTF',
  otf: 'OTF',
  shader: 'Shader',
  bin: 'Binary',
  jsontree: 'JSON (TypeTree)',
  ogv: 'OGV',
  binraw: 'Raw',
  raw: 'Raw',
}

export class ExportFormatUtils {
  static getDefaultFormats(className: string): ExportFormat[] {
    return DEFAULT_EXPORT_FORMATS[className] ?? ['raw']
  }

  static getRecommendedFormat(className: string): ExportFormat {
    return this.getDefaultFormats(className)[0]
  }

  static getLabel(format: ExportFormat): string {
    return EXPORT_FORMAT_LABELS[format] ?? format
  }
}
