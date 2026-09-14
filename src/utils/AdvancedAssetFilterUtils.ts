import type {
  AdvancedAssetFilterState,
  AdvancedAssetMatchMode,
  AdvancedAssetSizeUnit,
  AdvancedAssetSortDirection,
  AdvancedAssetSortField,
} from '../types'

const MATCH_MODES: AdvancedAssetMatchMode[] = ['contains', 'starts_with', 'ends_with', 'equals']
const SIZE_UNITS: AdvancedAssetSizeUnit[] = ['B', 'KB', 'MB']
const SORT_FIELDS: AdvancedAssetSortField[] = ['type', 'name', 'size', 'bundle', 'path_id']
const SORT_DIRECTIONS: AdvancedAssetSortDirection[] = ['asc', 'desc']

export function createDefaultAdvancedAssetFilter(): AdvancedAssetFilterState {
  return {
    classNames: [],
    nameQuery: '',
    nameMatchMode: 'contains',
    nameIncludeQueries: [],
    nameExcludeQuery: '',
    nameExcludeQueries: [],
    bundleQuery: '',
    pathIdQuery: '',
    minSizeText: '',
    maxSizeText: '',
    sizeUnit: 'MB',
    sortBy: 'type',
    sortDirection: 'asc',
  }
}

function normalizeString(value: unknown): string {
  return typeof value === 'string' ? value : ''
}

function normalizeTextList(value: unknown): string[] {
  if (!Array.isArray(value)) return []
  const seen = new Set<string>()
  const result: string[] = []
  for (const item of value) {
    if (typeof item !== 'string') continue
    const trimmed = item.trim()
    if (!trimmed) continue
    const key = trimmed.toLocaleLowerCase()
    if (seen.has(key)) continue
    seen.add(key)
    result.push(trimmed)
  }
  return result
}

function normalizeEnum<T extends string>(value: unknown, allowed: readonly T[], fallback: T): T {
  return typeof value === 'string' && (allowed as readonly string[]).includes(value) ? value as T : fallback
}

export function normalizeAdvancedAssetFilter(
  value: unknown,
  options: { allowBundleFilter?: boolean } = {},
): AdvancedAssetFilterState {
  const base = createDefaultAdvancedAssetFilter()
  if (!value || typeof value !== 'object') return base

  const filter = value as Partial<Record<keyof AdvancedAssetFilterState, unknown>>
  return {
    classNames: normalizeTextList(filter.classNames),
    nameQuery: normalizeString(filter.nameQuery),
    nameMatchMode: normalizeEnum(filter.nameMatchMode, MATCH_MODES, base.nameMatchMode),
    nameIncludeQueries: normalizeTextList(filter.nameIncludeQueries),
    nameExcludeQuery: normalizeString(filter.nameExcludeQuery),
    nameExcludeQueries: normalizeTextList(filter.nameExcludeQueries),
    bundleQuery: options.allowBundleFilter === false ? '' : normalizeString(filter.bundleQuery),
    pathIdQuery: normalizeString(filter.pathIdQuery),
    minSizeText: normalizeString(filter.minSizeText),
    maxSizeText: normalizeString(filter.maxSizeText),
    sizeUnit: normalizeEnum(filter.sizeUnit, SIZE_UNITS, base.sizeUnit),
    sortBy: normalizeEnum(filter.sortBy, SORT_FIELDS, base.sortBy),
    sortDirection: normalizeEnum(filter.sortDirection, SORT_DIRECTIONS, base.sortDirection),
  }
}
