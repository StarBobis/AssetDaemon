/**
 * Format utility class - provides formatting for byte sizes, durations, etc.
 *
 * Follows Soul.md convention: all methods must be class methods of a utility class; free functions are prohibited.
 * Low-level independent utility methods are categorized into corresponding XXXUtils.ts files under src/utils/.
 */

/**
 * Format utility class.
 *
 * Contains general-purpose methods for formatting byte sizes, time durations, etc.
 * All methods are static and do not involve business logic; they are pure utility methods.
 */
export class FormatUtils {
  /**
   * Formats a byte count into a human-readable string.
   *
   * Supports auto-scaling across B, KB, MB, GB, TB.
   * For example: 1024 -> "1.00 KB", 1536 -> "1.50 KB"
   *
   * @param bytes - Number of bytes
   * @returns Formatted string (with unit)
   */
  static formatSize(bytes: number): string {
    if (bytes === 0) return '0 B'
    const units = ['B', 'KB', 'MB', 'GB', 'TB']
    const k = 1024
    const i = Math.min(Math.floor(Math.log(bytes) / Math.log(k)), units.length - 1)
    const value = bytes / Math.pow(k, i)
    return `${value.toFixed(i === 0 ? 0 : 2)} ${units[i]}`
  }

  /**
   * Formats a millisecond duration into a human-readable time string.
   *
   * Output format: seconds part is an integer; when minutes are present, seconds are zero-padded.
   * For example: 1500 -> "1s", 65000 -> "1m 05s"
   *
   * @param ms - Number of milliseconds
   * @returns Formatted duration string
   */
  static formatDuration(ms: number): string {
    if (ms < 0) return '0s'
    const totalSeconds = Math.floor(ms / 1000)
    const minutes = Math.floor(totalSeconds / 60)
    const seconds = totalSeconds % 60
    if (minutes > 0) {
      return `${minutes}m ${String(seconds).padStart(2, '0')}s`
    }
    return `${seconds}s`
  }
}
