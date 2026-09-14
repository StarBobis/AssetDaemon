/**
 * WorkspacePathUtils - small path helpers used by workspace selection flows.
 *
 * Tauri can return Windows-style or POSIX-style paths depending on platform and API.
 * Keep path parsing here so UI flows do not hand-roll separator assumptions.
 */
export class WorkspacePathUtils {
  static parentDirectory(path: string): string {
    const normalized = path.trim()
    if (!normalized) return ''

    const withoutTrailingSeparators = normalized.replace(/[\\/]+$/, '')
    const lastForward = withoutTrailingSeparators.lastIndexOf('/')
    const lastBackslash = withoutTrailingSeparators.lastIndexOf('\\')
    const lastSeparator = Math.max(lastForward, lastBackslash)

    if (lastSeparator < 0) return ''
    if (lastSeparator === 0) return withoutTrailingSeparators.slice(0, 1)

    // Preserve Windows drive roots such as "C:\".
    if (
      lastSeparator === 2 &&
      withoutTrailingSeparators[1] === ':' &&
      /^[A-Za-z]$/.test(withoutTrailingSeparators[0])
    ) {
      return withoutTrailingSeparators.slice(0, 3)
    }

    return withoutTrailingSeparators.slice(0, lastSeparator)
  }
}
