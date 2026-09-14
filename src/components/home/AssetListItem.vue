<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { AssetClassificationUtils } from '../../utils/AssetClassificationUtils'
import { AssetDisplayUtils } from '../../utils/AssetDisplayUtils'
import type { AssetSummary } from '../../types'

const props = withDefaults(defineProps<{
  asset: AssetSummary
  assetKey: string
  active?: boolean
  selected?: boolean
  selectable?: boolean
  bundlePath?: string
  hierarchyPath?: string
  sizeText?: string
  showClass?: boolean
  showSize?: boolean
}>(), {
  active: false,
  selected: false,
  selectable: false,
  bundlePath: '',
  hierarchyPath: '',
  sizeText: '',
  showClass: false,
  showSize: true,
})

const emit = defineEmits<{
  click: [event: MouseEvent]
  contextmenu: [event: MouseEvent]
  checkboxClick: [event: MouseEvent]
}>()

const { t } = useI18n()

function displayName(): string {
  return AssetDisplayUtils.getAssetDisplayName(props.asset) || t('common.unnamed')
}

function isLikelyEmptyMesh(): boolean {
  return AssetClassificationUtils.isLikelyEmptyMesh(props.asset)
}

function shouldShowSize(): boolean {
  return props.showSize &&
    props.asset.class_name !== 'Texture2D' &&
    props.asset.class_name !== 'Sprite' &&
    props.asset.class_name !== 'SpriteMask'
}
</script>

<template>
  <div
    :class="[
      'asset-item',
      {
        'asset-item-active': active,
        'asset-item-selected': selected,
      },
    ]"
    :data-asset-key="assetKey"
    @click="emit('click', $event)"
    @contextmenu.prevent="emit('contextmenu', $event)"
  >
    <el-checkbox
      v-if="selectable"
      :model-value="selected"
      class="asset-checkbox"
      @click="emit('checkboxClick', $event)"
    />

    <div class="asset-item-main">
      <span class="asset-item-name">{{ displayName() }}</span>
      <span v-if="bundlePath" class="asset-item-container asset-bundle-path" :title="bundlePath">
        {{ bundlePath }}
      </span>
      <span v-if="hierarchyPath" class="asset-item-container asset-hierarchy-path" :title="hierarchyPath">
        {{ hierarchyPath }}
      </span>
    </div>

    <span class="asset-item-meta">
      <el-tooltip
        v-if="isLikelyEmptyMesh()"
        :content="t('assetPreview.likelyEmptyMeshDetail')"
        placement="top"
        popper-class="asset-empty-mesh-tooltip"
      >
        <span class="asset-item-badge">{{ t('assetList.likelyEmptyMesh') }}</span>
      </el-tooltip>
      <span v-if="showClass" class="asset-item-class">{{ asset.class_name }}</span>
      <span v-if="shouldShowSize()" class="asset-item-size">{{ sizeText }}</span>
    </span>
  </div>
</template>

<style scoped>
.asset-checkbox {
  width: 28px;
  height: 28px;
  margin-right: 4px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.asset-checkbox :deep(.el-checkbox__input) {
  width: 18px;
  height: 18px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.asset-checkbox :deep(.el-checkbox__inner) {
  width: 18px;
  height: 18px;
  border-radius: 4px;
}

.asset-checkbox :deep(.el-checkbox__inner::after) {
  display: none;
}

.asset-checkbox :deep(.el-checkbox__input.is-checked .el-checkbox__inner) {
  background-color: var(--el-color-success);
  border-color: var(--el-color-success);
  box-shadow: inset 0 0 0 3px color-mix(in srgb, var(--el-color-success) 82%, white);
}

.asset-checkbox :deep(.el-checkbox__input.is-focus .el-checkbox__inner) {
  border-color: var(--el-color-success);
}

.asset-checkbox :deep(.el-checkbox__input.is-checked + .el-checkbox__label) {
  color: inherit;
}

.asset-item-selected {
  background: color-mix(in srgb, var(--el-color-success) 14%, var(--app-surface));
  border-left-color: var(--el-color-success);
}

.asset-bundle-path,
.asset-hierarchy-path {
  opacity: 0.78;
  font-size: 11px;
}

.asset-item-class {
  max-width: 92px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--el-text-color-secondary);
  font-size: 11px;
}
</style>
