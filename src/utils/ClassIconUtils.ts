/**
 * Class icon utility class - maps Unity asset class names to Element Plus icon components.
 *
 * Follows Soul.md convention: all methods must be class methods of a utility class, no free functions.
 * Low-level independent utility methods are categorized under src/utils/ in corresponding XXXUtils.ts files.
 */

import {
  Document,
  VideoCameraFilled,
  PictureFilled,
  Files,
  Microphone,
} from '@element-plus/icons-vue'
import type { Component } from 'vue'

/**
 * Class icon utility class.
 *
 * Contains mapping from Unity asset class names to Element Plus icon components.
 * All methods are static methods, with no business logic - pure utility methods.
 */
export class ClassIconUtils {
  /**
   * Class name -> Element Plus icon component mapping table.
   */
  static classIconMap: Record<string, Component> = {
    Mesh: VideoCameraFilled,
    Texture2D: PictureFilled,
    Sprite: PictureFilled,
    AudioClip: Microphone,
    TextAsset: Files,
    MonoBehaviour: Document,
    Shader: Document,
    Material: Document,
    AnimationClip: VideoCameraFilled,
    Animator: VideoCameraFilled,
    GameObject: VideoCameraFilled,
  }

  /**
   * Get the corresponding Element Plus icon component for a class name.
   *
   * Returns the Document icon for unknown types.
   *
   * @param className - Unity asset class name (e.g., "Mesh", "Texture2D")
   * @returns Element Plus icon component
   */
  static getClassIcon(className: string): Component {
    return ClassIconUtils.classIconMap[className] || Document
  }
}
