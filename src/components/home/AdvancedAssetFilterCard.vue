<script setup lang="ts">
import { computed, nextTick, reactive } from 'vue'
import { Close, Refresh, Search } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import type { AdvancedAssetFilterState, MapAssetClassStat } from '../../types'
import { ClassIconUtils } from '../../utils/ClassIconUtils'
import CenteredFilterModal from './CenteredFilterModal.vue'

const { t } = useI18n()

const props = defineProps<{
  visible: boolean
  disabled?: boolean
  activeCount?: number
  showBundleFilter?: boolean
  typeStats: MapAssetClassStat[]
  modelValue: AdvancedAssetFilterState
}>()

const emit = defineEmits<{
  'update:visible': [value: boolean]
  'update:modelValue': [value: AdvancedAssetFilterState]
  apply: [value: AdvancedAssetFilterState]
  reset: []
}>()

const draft = reactive<AdvancedAssetFilterState>({
  ...props.modelValue,
  classNames: [...props.modelValue.classNames],
  nameIncludeQueries: [...(props.modelValue.nameIncludeQueries || [])],
  nameExcludeQueries: [...(props.modelValue.nameExcludeQueries || [])],
})

const popularTypes = computed(() => props.typeStats.slice(0, 12))
type ConditionKind = 'include' | 'exclude'

const pendingConditionText = reactive<Record<ConditionKind, string>>({
  include: '',
  exclude: '',
})

const editingCondition = reactive<{
  kind: ConditionKind | null
  index: number
  text: string
}>({
  kind: null,
  index: -1,
  text: '',
})

function syncDraft() {
  Object.assign(draft, {
    ...props.modelValue,
    classNames: [...props.modelValue.classNames],
    nameIncludeQueries: normalizeConditionList([
      props.modelValue.nameQuery,
      ...(props.modelValue.nameIncludeQueries || []),
    ]),
    nameExcludeQueries: normalizeConditionList([
      props.modelValue.nameExcludeQuery,
      ...(props.modelValue.nameExcludeQueries || []),
    ]),
    nameQuery: '',
    nameExcludeQuery: '',
  })
  pendingConditionText.include = ''
  pendingConditionText.exclude = ''
  cancelEditingCondition()
}

function updateVisible(value: boolean) {
  if (value && props.disabled) return
  if (value) syncDraft()
  emit('update:visible', value)
}

function openVisible() {
  updateVisible(true)
}

function commitAndApply() {
  if (editingCondition.kind !== null) finishEditingCondition()
  commitPendingConditions('include')
  commitPendingConditions('exclude')
  const nextValue = {
    ...draft,
    classNames: [...draft.classNames],
    nameQuery: '',
    nameIncludeQueries: normalizeConditionList(draft.nameIncludeQueries),
    nameExcludeQuery: '',
    nameExcludeQueries: normalizeConditionList(draft.nameExcludeQueries),
  }
  emit('update:modelValue', nextValue)
  emit('apply', nextValue)
  emit('update:visible', false)
}

function reset() {
  emit('reset')
  emit('update:visible', false)
}

function togglePopularType(name: string) {
  if (draft.classNames.includes(name)) {
    draft.classNames = draft.classNames.filter(item => item !== name)
  } else {
    draft.classNames = [...draft.classNames, name]
  }
}

function normalizeConditionList(values: Array<string | undefined>) {
  const seen = new Set<string>()
  const result: string[] = []
  for (const value of values) {
    if (!value) continue
    const trimmed = value.trim()
    if (!trimmed) continue
    const key = trimmed.toLocaleLowerCase()
    if (seen.has(key)) continue
    seen.add(key)
    result.push(trimmed)
  }
  return result
}

function getConditionList(kind: ConditionKind) {
  return kind === 'include' ? draft.nameIncludeQueries : draft.nameExcludeQueries
}

function setConditionList(kind: ConditionKind, values: string[]) {
  if (kind === 'include') {
    draft.nameIncludeQueries = values
  } else {
    draft.nameExcludeQueries = values
  }
}

function consumePendingConditions(kind: ConditionKind, commitRemainder = false) {
  const text = pendingConditionText[kind]
  if (!text) return
  const parts = text.split(/[,;，；]/)
  const hasDelimiter = parts.length > 1
  if (!hasDelimiter && !commitRemainder) return

  const tokens = commitRemainder ? parts : parts.slice(0, -1)
  const remainder = commitRemainder ? '' : parts[parts.length - 1]
  setConditionList(kind, normalizeConditionList([
    ...getConditionList(kind),
    ...tokens,
  ]))
  pendingConditionText[kind] = remainder
}

function commitPendingConditions(kind: ConditionKind) {
  consumePendingConditions(kind, true)
}

function removeCondition(kind: ConditionKind, index: number) {
  if (isEditingCondition(kind, index)) cancelEditingCondition()
  const nextValues = [...getConditionList(kind)]
  nextValues.splice(index, 1)
  setConditionList(kind, nextValues)
}

function startEditingCondition(kind: ConditionKind, index: number) {
  commitPendingConditions(kind)
  if (editingCondition.kind !== null) finishEditingCondition()
  const value = getConditionList(kind)[index]
  if (typeof value !== 'string') return
  editingCondition.kind = kind
  editingCondition.index = index
  editingCondition.text = value
  nextTick(() => {
    const input = document.querySelector<HTMLInputElement>(
      `[data-condition-edit="${kind}-${index}"]`,
    )
    input?.focus()
    input?.select()
  })
}

function finishEditingCondition() {
  if (editingCondition.kind === null || editingCondition.index < 0) return
  const kind = editingCondition.kind
  const nextValues = [...getConditionList(kind)]
  const index = editingCondition.index
  nextValues.splice(index, 1)
  const trimmed = editingCondition.text.trim()
  if (trimmed) {
    nextValues.splice(Math.min(index, nextValues.length), 0, trimmed)
  }
  setConditionList(kind, normalizeConditionList(nextValues))
  cancelEditingCondition()
}

function cancelEditingCondition() {
  editingCondition.kind = null
  editingCondition.index = -1
  editingCondition.text = ''
}

function isEditingCondition(kind: ConditionKind, index: number) {
  return editingCondition.kind === kind && editingCondition.index === index
}

function normalizeIncludeConditions() {
  commitPendingConditions('include')
  draft.nameIncludeQueries = normalizeConditionList(draft.nameIncludeQueries)
}

function normalizeExcludeConditions() {
  commitPendingConditions('exclude')
  draft.nameExcludeQueries = normalizeConditionList(draft.nameExcludeQueries)
}
</script>

<template>
  <button
    class="filter-icon-btn filter-text-btn"
    :class="{ active: visible || (activeCount ?? 0) > 0 }"
    :disabled="disabled"
    :title="t('filterCard.advancedTitle')"
    @click.stop="openVisible"
  >
    <span class="filter-btn-label">{{ t('filterCard.advancedTitle') }}</span>
    <span v-if="(activeCount ?? 0) > 0" class="filter-count">
      {{ activeCount }}
    </span>
  </button>

  <CenteredFilterModal
    :visible="visible"
    width="min(820px, calc(100vw - 32px))"
    @update:visible="updateVisible"
  >
    <div class="advanced-filter-card" @click.stop>
      <header class="advanced-filter-header">
        <div>
          <h3>{{ t('filterCard.advancedTitle') }}</h3>
          <p>{{ t('filterCard.advancedSubtitle') }}</p>
        </div>
        <div class="advanced-filter-actions">
          <el-button size="small" :icon="Refresh" @click="reset">{{ t('filterCard.reset') }}</el-button>
          <el-button size="small" type="primary" :icon="Search" @click="commitAndApply">{{ t('filterCard.apply') }}</el-button>
        </div>
      </header>

      <section class="filter-section">
        <div class="filter-section-title">{{ t('filterCard.assetType') }}</div>
        <el-select
          v-model="draft.classNames"
          multiple
          filterable
          :teleported="false"
          clearable
          :placeholder="t('filterCard.anyType')"
          class="wide-control"
        >
          <el-option
            v-for="stat in typeStats"
            :key="stat.name"
            :label="`${stat.name} (${stat.count})`"
            :value="stat.name"
          />
        </el-select>
        <div class="type-chip-row">
          <button
            v-for="stat in popularTypes"
            :key="stat.name"
            class="type-chip"
            :class="{ active: draft.classNames.includes(stat.name) }"
            @click="togglePopularType(stat.name)"
          >
            <el-icon :size="14"><component :is="ClassIconUtils.getClassIcon(stat.name)" /></el-icon>
            <span>{{ stat.name }}</span>
          </button>
        </div>
      </section>

      <section class="filter-grid">
        <div class="filter-field span-2">
          <span>{{ t('filterCard.nameContainerContainsAll') }}</span>
          <div class="inline-controls">
            <el-select v-model="draft.nameMatchMode" :teleported="false" class="mode-select">
              <el-option :label="t('filterCard.match.contains')" value="contains" />
              <el-option :label="t('filterCard.match.startsWith')" value="starts_with" />
              <el-option :label="t('filterCard.match.endsWith')" value="ends_with" />
              <el-option :label="t('filterCard.match.equals')" value="equals" />
            </el-select>
            <div class="editable-condition-field wide-control">
              <div
                v-for="(query, index) in draft.nameIncludeQueries"
                :key="`${query}-${index}`"
                class="condition-chip condition-chip--primary"
              >
                <input
                  v-if="isEditingCondition('include', index)"
                  v-model="editingCondition.text"
                  class="condition-edit-input"
                  :data-condition-edit="`include-${index}`"
                  @keydown.enter.stop.prevent="finishEditingCondition"
                  @keydown.esc.stop.prevent="cancelEditingCondition"
                  @blur="finishEditingCondition"
                >
                <button
                  v-else
                  type="button"
                  class="condition-chip-label"
                  :title="query"
                  @click="startEditingCondition('include', index)"
                >
                  {{ query }}
                </button>
                <button
                  type="button"
                  class="condition-chip-remove"
                  :title="t('common.remove')"
                  @mousedown.prevent
                  @click.stop="removeCondition('include', index)"
                >
                  <el-icon><Close /></el-icon>
                </button>
              </div>
              <input
                v-model="pendingConditionText.include"
                class="condition-input"
                :placeholder="draft.nameIncludeQueries.length ? '' : t('filterCard.requiredTermsPlaceholder')"
                @input="consumePendingConditions('include')"
                @keydown.enter.prevent="commitPendingConditions('include')"
                @blur="normalizeIncludeConditions"
              >
            </div>
          </div>
        </div>

        <div class="filter-field span-2">
          <span>{{ t('filterCard.nameContainerExcludeAny') }}</span>
          <div class="editable-condition-field wide-control">
            <div
              v-for="(query, index) in draft.nameExcludeQueries"
              :key="`${query}-${index}`"
              class="condition-chip condition-chip--warning"
            >
              <input
                v-if="isEditingCondition('exclude', index)"
                v-model="editingCondition.text"
                class="condition-edit-input"
                :data-condition-edit="`exclude-${index}`"
                @keydown.enter.stop.prevent="finishEditingCondition"
                @keydown.esc.stop.prevent="cancelEditingCondition"
                @blur="finishEditingCondition"
              >
              <button
                v-else
                type="button"
                class="condition-chip-label"
                :title="query"
                @click="startEditingCondition('exclude', index)"
              >
                {{ query }}
              </button>
              <button
                type="button"
                class="condition-chip-remove"
                :title="t('common.remove')"
                @mousedown.prevent
                @click.stop="removeCondition('exclude', index)"
              >
                <el-icon><Close /></el-icon>
              </button>
            </div>
            <input
              v-model="pendingConditionText.exclude"
              class="condition-input"
              :placeholder="draft.nameExcludeQueries.length ? '' : t('filterCard.forbiddenTermsPlaceholder')"
              @input="consumePendingConditions('exclude')"
              @keydown.enter.prevent="commitPendingConditions('exclude')"
              @blur="normalizeExcludeConditions"
            >
          </div>
        </div>

        <div v-if="showBundleFilter !== false" class="filter-field">
          <span>{{ t('filterCard.bundlePath') }}</span>
          <el-input v-model="draft.bundleQuery" clearable :placeholder="t('filterCard.containsPlaceholder')" />
        </div>

        <div class="filter-field">
          <span>{{ t('filterCard.pathId') }}</span>
          <el-input v-model="draft.pathIdQuery" clearable :placeholder="t('filterCard.pathIdPlaceholder')" />
        </div>

        <div class="filter-field">
          <span>{{ t('filterCard.minimumSize') }}</span>
          <el-input v-model="draft.minSizeText" clearable placeholder="0" />
        </div>

        <div class="filter-field">
          <span>{{ t('filterCard.maximumSize') }}</span>
          <div class="inline-controls">
            <el-input v-model="draft.maxSizeText" clearable :placeholder="t('filterCard.anyValue')" />
            <el-select v-model="draft.sizeUnit" :teleported="false" class="unit-select">
              <el-option label="B" value="B" />
              <el-option label="KB" value="KB" />
              <el-option label="MB" value="MB" />
            </el-select>
          </div>
        </div>

        <div class="filter-field">
          <span>{{ t('filterCard.sortBy') }}</span>
          <el-select v-model="draft.sortBy" :teleported="false">
            <el-option :label="t('filterCard.sort.type')" value="type" />
            <el-option :label="t('filterCard.sort.name')" value="name" />
            <el-option :label="t('filterCard.sort.size')" value="size" />
            <el-option :label="t('filterCard.sort.bundle')" value="bundle" />
            <el-option :label="t('filterCard.sort.pathId')" value="path_id" />
          </el-select>
        </div>

        <div class="filter-field">
          <span>{{ t('filterCard.direction') }}</span>
          <el-segmented
            v-model="draft.sortDirection"
            :options="[
              { label: t('filterCard.directionAsc'), value: 'asc' },
              { label: t('filterCard.directionDesc'), value: 'desc' },
            ]"
          />
        </div>
      </section>
    </div>
  </CenteredFilterModal>
</template>

<style scoped>
.advanced-filter-card {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 4px;
}

.advanced-filter-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  border-bottom: 1px solid var(--el-border-color-lighter);
  padding-bottom: 12px;
}

.advanced-filter-header h3 {
  margin: 0;
  font-size: 15px;
  line-height: 1.2;
}

.advanced-filter-header p {
  margin: 4px 0 0;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.advanced-filter-actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}

.filter-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.filter-section-title,
.filter-field > span {
  font-size: 12px;
  font-weight: 650;
  color: var(--el-text-color-primary);
}

.wide-control {
  width: 100%;
}

:deep(.el-input-tag.wide-control) {
  min-height: 32px;
}

:deep(.el-input-tag.wide-control .el-input-tag__inner) {
  align-items: flex-start;
  flex-wrap: wrap;
  row-gap: 5px;
}

.editable-condition-field {
  min-height: 32px;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 5px;
  padding: 4px 8px;
  border: 1px solid var(--el-border-color);
  border-radius: var(--el-border-radius-base);
  background: var(--el-fill-color-blank);
  transition: border-color 0.2s, box-shadow 0.2s;
}

.editable-condition-field:focus-within {
  border-color: var(--el-color-primary);
  box-shadow: 0 0 0 1px var(--el-color-primary-light-8);
}

.condition-chip {
  min-width: 0;
  max-width: 100%;
  height: 24px;
  display: inline-flex;
  align-items: center;
  gap: 3px;
  border-radius: 4px;
  padding: 0 3px 0 7px;
  font-size: 12px;
}

.condition-chip--primary {
  color: var(--el-color-primary);
  background: var(--el-color-primary-light-9);
  border: 1px solid var(--el-color-primary-light-7);
}

.condition-chip--warning {
  color: var(--el-color-warning-dark-2);
  background: var(--el-color-warning-light-9);
  border: 1px solid var(--el-color-warning-light-7);
}

.condition-chip-label,
.condition-chip-remove {
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  cursor: pointer;
}

.condition-chip-label {
  min-width: 0;
  max-width: min(260px, 100%);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 0;
}

.condition-chip-remove {
  width: 18px;
  height: 18px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border-radius: 50%;
}

.condition-chip-remove:hover {
  background: color-mix(in srgb, currentColor 14%, transparent);
}

.condition-input,
.condition-edit-input {
  height: 22px;
  min-width: 120px;
  flex: 1 1 140px;
  border: 0;
  outline: none;
  padding: 0;
  background: transparent;
  color: var(--el-text-color-primary);
  font: inherit;
}

.condition-edit-input {
  min-width: 72px;
  width: min(220px, 40vw);
  flex: 0 1 220px;
}

.condition-input::placeholder {
  color: var(--el-text-color-placeholder);
}

.type-chip-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.type-chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 26px;
  padding: 0 8px;
  border: 1px solid var(--el-border-color);
  border-radius: 4px;
  background: var(--el-bg-color);
  color: var(--el-text-color-regular);
  font-size: 12px;
  cursor: pointer;
}

.type-chip.active,
.type-chip:hover {
  border-color: var(--el-color-primary);
  color: var(--el-color-primary);
  background: var(--el-color-primary-light-9);
}

.filter-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.filter-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.span-2 {
  grid-column: span 2;
}

.inline-controls {
  display: flex;
  gap: 8px;
  min-width: 0;
}

.mode-select {
  width: 136px;
  flex-shrink: 0;
}

.unit-select {
  width: 82px;
  flex-shrink: 0;
}

@media (max-width: 900px) {
  .filter-grid {
    grid-template-columns: 1fr;
  }

  .span-2 {
    grid-column: span 1;
  }
}
</style>
