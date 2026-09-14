<script setup lang="ts">
/**
 * ExportDialog ->Batch Export Configuration Dialog
 *
 * Features:
 *  - Select output directory
 *  - Filter by asset type + format selection
 *  - Group options, overwrite options, report options
 *  - Export execution is reported through the log panel task area
 */

import { computed } from 'vue'
import { FolderOpened, Download } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import { useExport } from '../../composables/home/useExport'
import { ExportFormatUtils } from '../../utils/ExportFormatUtils'

const exp = useExport()
const { t } = useI18n()

interface TypeGroup {
  className: string
  count: number
  format: string
}

const typeGroups = computed<TypeGroup[]>(() => {
  const groups: Record<string, number> = {}
  for (const a of exp.selectedAssets.value) {
    groups[a.class_name] = (groups[a.class_name] || 0) + 1
  }
  return Object.entries(groups).map(([className, count]) => ({
    className,
    count,
    format: exp.formatOverrides.value[className] || '',
  }))
})

const totalSelected = computed(() =>
  typeGroups.value.filter(g => exp.formatOverrides.value[g.className]).reduce((s, g) => s + g.count, 0)
)

const isRunning = computed(() => exp.isExporting.value)

function getFormats(className: string): string[] {
  return ExportFormatUtils.getDefaultFormats(className)
}

function getFormatLabel(format: string): string {
  return ExportFormatUtils.getLabel(format as import('../../types').ExportFormat)
}

function handleClosed() {
  if (!exp.isExporting.value) {
    exp.resetExport()
  }
}
</script>

<template>
  <el-dialog
    :model-value="exp.showDialog.value"
    :close-on-click-modal="!isRunning"
    :close-on-press-escape="!isRunning"
    :show-close="!isRunning"
    width="560px"
    align-center
    @update:model-value="exp.closeDialog()"
    @closed="handleClosed"
  >
    <template #header>
      <span class="dialog-title">{{ t('exportDialog.title') }}</span>
    </template>

    <!-- ============================================================ -->
    <!-- Configuration Area -->
    <!-- ============================================================ -->
    <div class="export-body">
      <!-- Output Directory -->
      <div class="field">
        <label class="field-label">{{ t('exportDialog.outputDirectory') }}</label>
        <div class="field-row">
          <el-input
            :model-value="exp.outputDir.value"
            :placeholder="t('exportDialog.selectDirectory')"
            readonly
            size="small"
          />
          <el-button size="small" @click="exp.browseOutputDir()">
            <el-icon><FolderOpened /></el-icon>
            {{ t('common.browse') }}
          </el-button>
        </div>
      </div>

      <!-- Asset Type List -->
      <div class="field">
        <label class="field-label">
          {{ t('exportDialog.assetTypes', { count: totalSelected }) }}
        </label>
        <div class="type-list">
          <div
            v-for="g in typeGroups"
            :key="g.className"
            class="type-row"
          >
            <el-checkbox
              :model-value="!!exp.formatOverrides.value[g.className]"
              size="small"
              @change="exp.toggleType(g.className)"
            >
              {{ g.className }}
            </el-checkbox>
            <span class="type-count">({{ g.count }})</span>
            <el-select
              v-if="exp.formatOverrides.value[g.className]"
              :model-value="exp.formatOverrides.value[g.className]"
              size="small"
              class="format-select"
              @update:model-value="(v: string | number | boolean) => exp.setFormat(g.className, v as import('../../types').ExportFormat)"
            >
              <el-option
                v-for="fmt in getFormats(g.className)"
                :key="fmt"
                :label="getFormatLabel(fmt)"
                :value="fmt"
              />
            </el-select>
          </div>
        </div>
      </div>

      <!-- Options -->
      <div class="field">
        <label class="field-label">{{ t('exportDialog.options') }}</label>
        <div class="options-grid">
          <div class="option-row">
            <span class="option-label">{{ t('exportDialog.groupBy') }}</span>
            <el-radio-group v-model="exp.groupBy.value" size="small">
              <el-radio-button value="none">{{ t('exportDialog.groupNone') }}</el-radio-button>
              <el-radio-button value="bytype">{{ t('exportDialog.groupByType') }}</el-radio-button>
              <el-radio-button value="bycontainer">{{ t('exportDialog.groupByContainer') }}</el-radio-button>
            </el-radio-group>
          </div>
          <div class="option-row">
            <el-checkbox v-model="exp.overwriteExisting.value" size="small">
              {{ t('exportDialog.overwriteExisting') }}
            </el-checkbox>
          </div>
          <div class="option-row">
            <el-checkbox v-model="exp.generateReport.value" size="small">
              {{ t('exportDialog.generateReport') }}
            </el-checkbox>
          </div>
          <div class="option-row">
            <el-checkbox v-model="exp.splitSubmeshes.value" size="small">
              {{ t('exportDialog.splitSubmeshes') }}
            </el-checkbox>
          </div>
        </div>
      </div>

      <div class="field">
        <label class="field-label">{{ t('exportDialog.heuristicContent') }}</label>
        <p class="field-hint">{{ t('exportDialog.heuristicContentHint') }}</p>
        <div class="heuristic-grid">
          <el-checkbox v-model="exp.includeMaterials.value" size="small">
            {{ t('exportDialog.includeMaterials') }}
          </el-checkbox>
          <el-checkbox v-model="exp.includeTextures.value" size="small">
            {{ t('exportDialog.includeTextures') }}
          </el-checkbox>
          <el-checkbox v-model="exp.includeSkeleton.value" size="small">
            {{ t('exportDialog.includeSkeleton') }}
          </el-checkbox>
          <el-checkbox v-model="exp.includeAnimations.value" size="small">
            {{ t('exportDialog.includeAnimations') }}
          </el-checkbox>
        </div>
      </div>
    </div>

    <!-- ============================================================ -->
    <!-- Footer -->
    <!-- ============================================================ -->
    <template #footer>
      <div class="dialog-footer">
        <el-button size="small" @click="exp.closeDialog()">{{ t('common.cancel') }}</el-button>
        <el-button
          type="primary"
          size="small"
          :disabled="!exp.outputDir.value || totalSelected === 0"
          @click="exp.startExport()"
        >
          <el-icon><Download /></el-icon>
          {{ t('exportDialog.exportAssets', { count: totalSelected }) }}
        </el-button>
      </div>
    </template>
  </el-dialog>
</template>

<style scoped>
.dialog-title {
  font-size: 16px;
  font-weight: 600;
}

.export-body {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.field-label {
  display: block;
  font-size: 12px;
  font-weight: 600;
  color: var(--el-text-color-secondary);
  margin-bottom: 6px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.field-hint {
  margin: -2px 0 8px;
  color: var(--el-text-color-secondary);
  font-size: 11px;
  line-height: 1.45;
}

.field-row {
  display: flex;
  gap: 8px;
}

.type-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 220px;
  overflow-y: auto;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
  padding: 8px;
}

.type-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 0;
}

.type-count {
  font-size: 11px;
  color: var(--el-text-color-placeholder);
  min-width: 30px;
}

.format-select {
  width: 110px;
  margin-left: auto;
}

.options-grid {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.heuristic-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 6px 12px;
  padding: 9px 10px;
  border: 1px solid var(--app-border-soft);
  border-radius: 10px;
  background: color-mix(in srgb, var(--el-color-warning) 7%, var(--app-surface));
}

.option-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.option-label {
  font-size: 12px;
  width: 60px;
  color: var(--el-text-color-secondary);
}

.dialog-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
