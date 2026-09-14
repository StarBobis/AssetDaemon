import { describe, expect, it } from 'vitest'
import { WorkspacePathUtils } from './WorkspacePathUtils'

describe('WorkspacePathUtils.parentDirectory', () => {
  it('returns the parent for Windows paths', () => {
    expect(WorkspacePathUtils.parentDirectory('D:\\Game\\Bundles\\hero.bundle')).toBe('D:\\Game\\Bundles')
  })

  it('returns the parent for POSIX paths returned by cross-platform APIs', () => {
    expect(WorkspacePathUtils.parentDirectory('/mnt/game/bundles/hero.bundle')).toBe('/mnt/game/bundles')
  })

  it('preserves Windows drive roots', () => {
    expect(WorkspacePathUtils.parentDirectory('D:\\hero.bundle')).toBe('D:\\')
  })

  it('returns empty string when no parent can be inferred', () => {
    expect(WorkspacePathUtils.parentDirectory('hero.bundle')).toBe('')
  })
})
