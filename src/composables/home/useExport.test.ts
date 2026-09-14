import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useExport } from './useExport'

const mocks = vi.hoisted(() => ({
  mockStoreGet: vi.fn().mockResolvedValue(undefined),
}))
const { mockStoreGet } = mocks

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  Channel: vi.fn(),
}))

vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn(),
}))

vi.mock('@tauri-apps/plugin-store', () => ({
  load: vi.fn().mockResolvedValue({
    get: mocks.mockStoreGet,
    set: vi.fn().mockResolvedValue(undefined),
    save: vi.fn().mockResolvedValue(undefined),
  }),
}))

vi.mock('@tauri-apps/plugin-opener', () => ({
  revealItemInDir: vi.fn(),
  openPath: vi.fn(),
}))

vi.mock('../../i18n', () => ({
  i18n: {
    global: {
      t: (key: string) => key,
    },
  },
}))

describe('useExport', () => {
  beforeEach(() => {
    mockStoreGet.mockReset()
    mockStoreGet.mockResolvedValue(undefined)
    useExport().resetExport()
  })

  it('selects raw by default for asset types missing from the explicit format table', () => {
    const exp = useExport()

    exp.openExportDialog([{
      bundle_path: 'bundle-a',
      path_id: '42',
      class_name: 'CustomUnityAsset',
      asset_name: 'custom',
    }])

    expect(exp.showDialog.value).toBe(true)
    expect(exp.formatOverrides.value).toEqual({
      CustomUnityAsset: 'raw',
    })
  })

  it('restores raw when re-enabling an unknown asset type', () => {
    const exp = useExport()

    exp.openExportDialog([{
      bundle_path: 'bundle-a',
      path_id: '42',
      class_name: 'CustomUnityAsset',
      asset_name: 'custom',
    }])
    exp.toggleType('CustomUnityAsset')
    exp.toggleType('CustomUnityAsset')

    expect(exp.formatOverrides.value).toEqual({
      CustomUnityAsset: 'raw',
    })
  })

  it('selects glb by default for GameObject model exports', () => {
    const exp = useExport()

    exp.openExportDialog([{
      bundle_path: 'bundle-a',
      path_id: '99',
      class_name: 'GameObject',
      asset_name: 'hero.prefab',
    }])

    expect(exp.formatOverrides.value).toEqual({
      GameObject: 'glb',
    })
  })

  it('does not let old saved GameObject raw preference override model export default', async () => {
    mockStoreGet.mockImplementation(async (key: string) => {
      if (key === 'export-format-overrides') return { GameObject: 'raw' }
      return undefined
    })
    const exp = useExport()

    exp.openExportDialog([{
      bundle_path: 'bundle-a',
      path_id: '99',
      class_name: 'GameObject',
      asset_name: 'hero.prefab',
    }])
    await Promise.resolve()
    await Promise.resolve()

    expect(exp.formatOverrides.value).toEqual({
      GameObject: 'glb',
    })
  })

})
